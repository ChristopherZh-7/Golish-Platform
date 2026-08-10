//! Server-derived, operation-wide report-input authority sets.
//!
//! A report revision can cover many organizations and every current hypothesis
//! adjudication, terminal or nonterminal. This module freezes each member in canonical
//! order; no caller supplies an authority id, count, hash, or timestamp.

use chrono::{DateTime, Utc};
use golish_reporting_domain::{
    AllFreshToolTruthAuthorityBundleRefV1, LegacyCoverageLimitationCode, LegacyReportInputSealV1,
    ReportInputSealV1, ReportSourceSnapshot, ReportToolTruthAuthoritySetRefV1,
    RevisionAdjudicationAuthorityMemberV1, RevisionAdjudicationAuthoritySetRefV1,
    RevisionAdjudicationOutcomeV1, RevisionAdjudicationReportInputSealV1, WaveTerminalReceiptRefV1,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

fn conflict(code: &'static str) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(code))
}

fn digest_json(value: &Value) -> Result<[u8; 32]> {
    Ok(Sha256::digest(serde_json::to_vec(value)?).into())
}

fn digest_serializable(value: &impl serde::Serialize) -> Result<[u8; 32]> {
    Ok(Sha256::digest(serde_json::to_vec(value)?).into())
}

fn decode_tagged_hash(value: &str) -> Result<[u8; 32]> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(conflict("REPORT_INPUT_AUTHORITY_HASH_INVALID"));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(conflict("REPORT_INPUT_AUTHORITY_HASH_INVALID"));
    }
    let mut output = [0_u8; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        let high = (pair[0] as char)
            .to_digit(16)
            .ok_or_else(|| conflict("REPORT_INPUT_AUTHORITY_HASH_INVALID"))?;
        let low = (pair[1] as char)
            .to_digit(16)
            .ok_or_else(|| conflict("REPORT_INPUT_AUTHORITY_HASH_INVALID"))?;
        output[index] = u8::try_from((high << 4) | low)
            .map_err(|_| conflict("REPORT_INPUT_AUTHORITY_HASH_INVALID"))?;
    }
    Ok(output)
}

fn decode_bare_hash(value: &str) -> Result<[u8; 32]> {
    decode_tagged_hash(&format!("sha256:{value}"))
}

fn aggregate_hash(domain: &'static str, members: &[[u8; 32]]) -> Result<[u8; 32]> {
    digest_json(&json!({ "domain": domain, "members": members }))
}

#[derive(sqlx::FromRow)]
struct ToolTruthBundleRow {
    organization_id: Uuid,
    id: Uuid,
    relevant_root_count: i64,
    relevant_root_set_hash: String,
    member_count: i64,
    member_set_hash: String,
    semantic_authority_bundle_hash: String,
    freshness_attestation_bundle_hash: String,
    temporal_validity_bundle_hash: String,
    temporal_validity_policy_set_hash: String,
    target_state_epoch_set_hash: String,
    observation_window_started_at: Option<DateTime<Utc>>,
    observation_window_completed_at: Option<DateTime<Utc>>,
    effective_valid_until: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct PersistedReportInputAuthorityRow {
    typed_seal: Value,
    authority_contract: String,
    tool_truth_authority_set_id: Uuid,
    revision_adjudication_authority_set_id: Option<Uuid>,
    legacy_report_authority_seal_id: Option<Uuid>,
    source_member_count: i64,
    source_set_hash: Vec<u8>,
    report_input_hash: Vec<u8>,
    effective_valid_until: DateTime<Utc>,
    revision_source_set_hash: String,
    manifest_count: i64,
    observed_at: DateTime<Utc>,
    invalidated: bool,
}

fn tool_truth_ref(row: &ToolTruthBundleRow) -> Result<AllFreshToolTruthAuthorityBundleRefV1> {
    let relevant_root_count = u64::try_from(row.relevant_root_count)
        .map_err(|_| conflict("REPORT_INPUT_TOOL_TRUTH_COUNT_INVALID"))?;
    let relevant_member_count = u64::try_from(row.member_count)
        .map_err(|_| conflict("REPORT_INPUT_TOOL_TRUTH_COUNT_INVALID"))?;
    let relevant_root_set_hash = decode_tagged_hash(&row.relevant_root_set_hash)?;
    let relevant_member_set_hash = decode_tagged_hash(&row.member_set_hash)?;
    let semantic_authority_hash = decode_tagged_hash(&row.semantic_authority_bundle_hash)?;
    let freshness_authority_hash = decode_tagged_hash(&row.freshness_attestation_bundle_hash)?;
    let temporal_validity_hash = decode_tagged_hash(&row.temporal_validity_bundle_hash)?;
    let epoch_hash = decode_tagged_hash(&row.target_state_epoch_set_hash)?;
    let observation_window_hash = digest_json(&json!({
        "domain":"report_tool_truth_observation_window.v1",
        "started_at":row.observation_window_started_at,
        "completed_at":row.observation_window_completed_at,
    }))?;
    let effective_validity_hash = digest_json(&json!({
        "domain":"report_tool_truth_effective_validity.v1",
        "temporal_policy_set_hash":row.temporal_validity_policy_set_hash,
        "effective_valid_until":row.effective_valid_until,
    }))?;
    let bundle_hash = digest_json(&json!({
        "domain":"report_tool_truth_authority_bundle_ref.v1",
        "bundle_id":row.id,
        "organization_id":row.organization_id,
        "relevant_root_count":relevant_root_count,
        "relevant_root_set_hash":relevant_root_set_hash,
        "relevant_member_count":relevant_member_count,
        "relevant_member_set_hash":relevant_member_set_hash,
        "semantic_authority_hash":semantic_authority_hash,
        "freshness_authority_hash":freshness_authority_hash,
        "temporal_validity_hash":temporal_validity_hash,
        "epoch_hash":epoch_hash,
        "observation_window_hash":observation_window_hash,
        "effective_validity_hash":effective_validity_hash,
        "effective_valid_until":row.effective_valid_until,
    }))?;
    Ok(AllFreshToolTruthAuthorityBundleRefV1 {
        bundle_id: row.id,
        bundle_hash,
        relevant_root_count,
        relevant_root_set_hash,
        relevant_member_count,
        relevant_member_set_hash,
        semantic_authority_hash,
        freshness_authority_hash,
        temporal_validity_hash,
        epoch_hash,
        observation_window_hash,
        effective_validity_hash,
        effective_valid_until: row.effective_valid_until,
    })
}

async fn seal_report_tool_truth_set_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    revision_id: Uuid,
) -> Result<ReportToolTruthAuthoritySetRefV1> {
    let organizations: Vec<Uuid> = sqlx::query_scalar(
        r#"WITH scope AS (
               SELECT id FROM operation_org_scope_snapshots
                WHERE operation_id=$1 AND sealed_at IS NOT NULL
                ORDER BY sealed_at DESC,id DESC LIMIT 1
           )
           SELECT unit.organization_id
             FROM operation_org_scope_units unit JOIN scope ON scope.id=unit.snapshot_id
            ORDER BY unit.ordinal,unit.organization_id"#,
    )
    .bind(operation_id)
    .fetch_all(&mut **tx)
    .await?;
    if organizations.is_empty() {
        return Err(conflict("REPORT_INPUT_TOOL_TRUTH_SCOPE_EMPTY"));
    }
    let mut rows = Vec::with_capacity(organizations.len());
    for organization_id in organizations {
        let stable_request_id = Uuid::new_v5(
            &revision_id,
            format!("report-tool-truth:{organization_id}").as_bytes(),
        );
        let row = sqlx::query_as::<_, ToolTruthBundleRow>(
            r#"SELECT organization_id,id,relevant_root_count,relevant_root_set_hash,
                      member_count,member_set_hash,semantic_authority_bundle_hash,
                      freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
                      temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                      observation_window_started_at,observation_window_completed_at,
                      effective_valid_until
                 FROM tool_truth_authority_bundle_seals
                WHERE operation_id=$1 AND organization_id=$2
                  AND consumer_kind='current_report' AND stable_consumer_request_id=$3
                  AND sealed_at IS NOT NULL AND relevant_root_count=3 AND member_count=3
                  AND consistent_fresh_count=4 AND stale_or_invalid_count=0
                  AND transaction_timestamp()<=effective_valid_until
                FOR SHARE"#,
        )
        .bind(operation_id)
        .bind(organization_id)
        .bind(stable_request_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict("REPORT_INPUT_TOOL_TRUTH_AUTHORITY_MISSING"))?;
        rows.push(row);
    }
    rows.sort_by_key(|row| row.organization_id);
    let authority_set_id = Uuid::new_v5(&revision_id, b"report-tool-truth-authority-set.v1");
    let mut typed_members = Vec::with_capacity(rows.len());
    let mut member_hashes = Vec::with_capacity(rows.len());
    let mut earliest = None::<DateTime<Utc>>;
    for row in &rows {
        let authority = tool_truth_ref(row)?;
        let typed_member = StoredReportToolTruthMemberV1 {
            schema: "report_tool_truth_authority_member.v1".to_owned(),
            organization_id: row.organization_id,
            authority: authority.clone(),
        };
        let member_hash = digest_serializable(&typed_member)?;
        earliest = Some(
            earliest
                .map(|current| current.min(authority.effective_valid_until))
                .unwrap_or(authority.effective_valid_until),
        );
        typed_members.push((row, typed_member, member_hash));
        member_hashes.push(member_hash);
    }
    let authority_set_hash = aggregate_hash("report_tool_truth_authority_set.v1", &member_hashes)?;
    let earliest_effective_valid_until =
        earliest.ok_or_else(|| conflict("REPORT_INPUT_TOOL_TRUTH_SCOPE_EMPTY"))?;
    let member_count =
        i64::try_from(rows.len()).map_err(|_| conflict("REPORT_INPUT_TOOL_TRUTH_COUNT_INVALID"))?;
    sqlx::query(
        r#"INSERT INTO report_input_tool_truth_authority_sets(
               authority_set_id,revision_id,operation_id,authority_member_count,
               authority_set_hash,earliest_effective_valid_until
           ) VALUES($1,$2,$3,$4,$5,$6)"#,
    )
    .bind(authority_set_id)
    .bind(revision_id)
    .bind(operation_id)
    .bind(member_count)
    .bind(authority_set_hash.as_slice())
    .bind(earliest_effective_valid_until)
    .execute(&mut **tx)
    .await?;
    for (ordinal, (row, typed_member, member_hash)) in typed_members.into_iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO report_input_tool_truth_authority_members(
                   authority_set_id,revision_id,operation_id,ordinal,organization_id,
                   tool_truth_authority_bundle_id,typed_member,effective_valid_until,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
        )
        .bind(authority_set_id)
        .bind(revision_id)
        .bind(operation_id)
        .bind(i32::try_from(ordinal).map_err(|_| conflict("REPORT_INPUT_AUTHORITY_COUNT_INVALID"))?)
        .bind(row.organization_id)
        .bind(row.id)
        .bind(serde_json::to_value(typed_member)?)
        .bind(row.effective_valid_until)
        .bind(member_hash.as_slice())
        .execute(&mut **tx)
        .await?;
    }
    Ok(ReportToolTruthAuthoritySetRefV1 {
        authority_set_id,
        authority_member_count: u64::try_from(member_count)
            .map_err(|_| conflict("REPORT_INPUT_TOOL_TRUTH_COUNT_INVALID"))?,
        authority_set_hash,
        earliest_effective_valid_until,
    })
}

#[derive(Deserialize, Serialize)]
struct StoredReportToolTruthMemberV1 {
    schema: String,
    organization_id: Uuid,
    authority: AllFreshToolTruthAuthorityBundleRefV1,
}

#[derive(sqlx::FromRow)]
struct StoredToolTruthMemberRow {
    ordinal: i32,
    organization_id: Uuid,
    tool_truth_authority_bundle_id: Uuid,
    typed_member: Value,
    effective_valid_until: DateTime<Utc>,
    member_hash: Vec<u8>,
}

async fn tool_truth_bundle_members_are_live_on(
    connection: &mut PgConnection,
    bundle_id: Uuid,
    operation_id: Uuid,
    organization_id: Uuid,
) -> Result<bool> {
    let live = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM tool_truth_authority_bundle_seals bundle
                WHERE bundle.id=$1 AND bundle.operation_id=$2
                  AND bundle.organization_id=$3 AND bundle.sealed_at IS NOT NULL
                  AND bundle.member_count=(
                      SELECT COUNT(*) FROM tool_truth_authority_bundle_members bundle_member
                       WHERE bundle_member.bundle_seal_id=bundle.id)
                  AND NOT EXISTS(
                      SELECT 1
                        FROM tool_truth_authority_bundle_members bundle_member
                        JOIN tool_truth_authority_set_members authority_member
                          ON authority_member.authority_set_id=
                             bundle_member.authority_set_seal_id
                        JOIN capability_execution_receipts receipt
                          ON receipt.id=authority_member.receipt_id
                         AND receipt.execution_authority_id=
                             authority_member.execution_authority_id
                       WHERE bundle_member.bundle_seal_id=bundle.id
                         AND (receipt.reconciliation_state<>'consistent'
                              OR receipt.current_semantic_reconciliation_id
                                 IS DISTINCT FROM authority_member.reconciliation_id
                              OR receipt.current_semantic_authority_version
                                 IS DISTINCT FROM authority_member.semantic_authority_version
                              OR receipt.current_semantic_reconciliation_hash
                                 IS DISTINCT FROM authority_member.semantic_hash)
                  )
           )"#,
    )
    .bind(bundle_id)
    .bind(operation_id)
    .bind(organization_id)
    .fetch_one(&mut *connection)
    .await?;
    Ok(live)
}

async fn validate_report_tool_truth_set_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    revision_id: Uuid,
    authority: &ReportToolTruthAuthoritySetRefV1,
    observed_at: DateTime<Utc>,
) -> Result<()> {
    let header: Option<(i64, Vec<u8>, DateTime<Utc>)> = sqlx::query_as(
        r#"SELECT authority_member_count,authority_set_hash,earliest_effective_valid_until
             FROM report_input_tool_truth_authority_sets
            WHERE authority_set_id=$1 AND revision_id=$2 AND operation_id=$3 FOR SHARE"#,
    )
    .bind(authority.authority_set_id)
    .bind(revision_id)
    .bind(operation_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some((stored_count, stored_hash, stored_earliest)) = header else {
        return Err(conflict("REPORT_INPUT_TOOL_TRUTH_SET_MISSING"));
    };
    if u64::try_from(stored_count).ok() != Some(authority.authority_member_count)
        || stored_hash.as_slice() != authority.authority_set_hash
        || stored_earliest != authority.earliest_effective_valid_until
        || stored_earliest <= observed_at
    {
        return Err(conflict("REPORT_INPUT_TOOL_TRUTH_SET_STALE"));
    }
    let current_scope_organizations: Vec<Uuid> = sqlx::query_scalar(
        r#"WITH scope AS (
               SELECT id FROM operation_org_scope_snapshots
                WHERE operation_id=$1 AND sealed_at IS NOT NULL
                ORDER BY sealed_at DESC,id DESC LIMIT 1
           )
           SELECT unit.organization_id
             FROM operation_org_scope_units unit JOIN scope ON scope.id=unit.snapshot_id
            ORDER BY unit.organization_id"#,
    )
    .bind(operation_id)
    .fetch_all(&mut *connection)
    .await?;
    let members = sqlx::query_as::<_, StoredToolTruthMemberRow>(
        r#"SELECT ordinal,organization_id,tool_truth_authority_bundle_id,typed_member,
                  effective_valid_until,member_hash
             FROM report_input_tool_truth_authority_members
            WHERE authority_set_id=$1 ORDER BY ordinal FOR SHARE"#,
    )
    .bind(authority.authority_set_id)
    .fetch_all(&mut *connection)
    .await?;
    if members.len() != usize::try_from(stored_count).unwrap_or(usize::MAX)
        || members
            .iter()
            .map(|member| member.organization_id)
            .collect::<Vec<_>>()
            != current_scope_organizations
    {
        return Err(conflict("REPORT_INPUT_TOOL_TRUTH_SCOPE_STALE"));
    }
    let mut hashes = Vec::with_capacity(members.len());
    let mut earliest = None::<DateTime<Utc>>;
    for (expected_ordinal, member) in members.iter().enumerate() {
        if usize::try_from(member.ordinal).ok() != Some(expected_ordinal)
            || member.effective_valid_until <= observed_at
        {
            return Err(conflict("REPORT_INPUT_TOOL_TRUTH_MEMBER_STALE"));
        }
        let typed: StoredReportToolTruthMemberV1 =
            serde_json::from_value(member.typed_member.clone())
                .map_err(|_| conflict("REPORT_INPUT_TOOL_TRUTH_MEMBER_CORRUPT"))?;
        let stable_request_id = Uuid::new_v5(
            &revision_id,
            format!("report-tool-truth:{}", member.organization_id).as_bytes(),
        );
        let live_bundle = sqlx::query_as::<_, ToolTruthBundleRow>(
            r#"SELECT organization_id,id,relevant_root_count,relevant_root_set_hash,
                      member_count,member_set_hash,semantic_authority_bundle_hash,
                      freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
                      temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                      observation_window_started_at,observation_window_completed_at,
                      effective_valid_until
                 FROM tool_truth_authority_bundle_seals
                WHERE id=$1 AND operation_id=$2 AND organization_id=$3
                  AND consumer_kind='current_report' AND stable_consumer_request_id=$4
                  AND sealed_at IS NOT NULL AND relevant_root_count=3 AND member_count=3
                  AND consistent_fresh_count=4 AND stale_or_invalid_count=0
                  AND transaction_timestamp()<=effective_valid_until
                FOR SHARE"#,
        )
        .bind(member.tool_truth_authority_bundle_id)
        .bind(operation_id)
        .bind(member.organization_id)
        .bind(stable_request_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or_else(|| conflict("REPORT_INPUT_TOOL_TRUTH_MEMBER_STALE"))?;
        if !tool_truth_bundle_members_are_live_on(
            connection,
            live_bundle.id,
            operation_id,
            member.organization_id,
        )
        .await?
        {
            return Err(conflict("REPORT_INPUT_TOOL_TRUTH_MEMBER_STALE"));
        }
        let live_authority = tool_truth_ref(&live_bundle)?;
        let recomputed_hash = digest_serializable(&typed)?;
        if typed.schema != "report_tool_truth_authority_member.v1"
            || typed.organization_id != member.organization_id
            || typed.authority != live_authority
            || typed.authority.bundle_id != member.tool_truth_authority_bundle_id
            || typed.authority.effective_valid_until != member.effective_valid_until
            || recomputed_hash.as_slice() != member.member_hash
        {
            return Err(conflict("REPORT_INPUT_TOOL_TRUTH_MEMBER_CORRUPT"));
        }
        hashes.push(recomputed_hash);
        earliest = Some(
            earliest
                .map(|current| current.min(member.effective_valid_until))
                .unwrap_or(member.effective_valid_until),
        );
    }
    if aggregate_hash("report_tool_truth_authority_set.v1", &hashes)?
        != authority.authority_set_hash
        || earliest != Some(authority.earliest_effective_valid_until)
    {
        return Err(conflict("REPORT_INPUT_TOOL_TRUTH_SET_CORRUPT"));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct RevisionAuthorityRow {
    organization_id: Uuid,
    hypothesis_revision_id: Uuid,
    generation_seal_id: Uuid,
    generation_seal_hash: String,
    verification_plan_seal_id: Uuid,
    verification_plan_seal_hash: String,
    proof_path_set_hash: String,
    claim_component_set_hash: String,
    revision_adjudication_id: Uuid,
    revision_adjudication_hash: String,
    adjudication_outcome: String,
    revision_terminal_decision_id: Option<Uuid>,
    revision_terminal_decision_hash: Option<String>,
    latest_objective_outcome_member_count: i64,
    latest_objective_outcome_set_hash: String,
    wave_coverage_receipt_id: Uuid,
    wave_coverage_receipt_hash: String,
    coverage_membership_hash: String,
    residual_membership_hash: String,
    consolidation_receipt_id: Option<Uuid>,
    consolidation_receipt_hash: Option<String>,
    fixed_point_receipt_id: Option<Uuid>,
    fixed_point_receipt_hash: Option<String>,
    authority_effective_valid_until: DateTime<Utc>,
    bundle_organization_id: Uuid,
    id: Uuid,
    relevant_root_count: i64,
    relevant_root_set_hash: String,
    member_count: i64,
    member_set_hash: String,
    semantic_authority_bundle_hash: String,
    freshness_attestation_bundle_hash: String,
    temporal_validity_bundle_hash: String,
    temporal_validity_policy_set_hash: String,
    target_state_epoch_set_hash: String,
    observation_window_started_at: Option<DateTime<Utc>>,
    observation_window_completed_at: Option<DateTime<Utc>>,
    effective_valid_until: DateTime<Utc>,
}

fn revision_bundle_row(row: &RevisionAuthorityRow) -> ToolTruthBundleRow {
    ToolTruthBundleRow {
        organization_id: row.bundle_organization_id,
        id: row.id,
        relevant_root_count: row.relevant_root_count,
        relevant_root_set_hash: row.relevant_root_set_hash.clone(),
        member_count: row.member_count,
        member_set_hash: row.member_set_hash.clone(),
        semantic_authority_bundle_hash: row.semantic_authority_bundle_hash.clone(),
        freshness_attestation_bundle_hash: row.freshness_attestation_bundle_hash.clone(),
        temporal_validity_bundle_hash: row.temporal_validity_bundle_hash.clone(),
        temporal_validity_policy_set_hash: row.temporal_validity_policy_set_hash.clone(),
        target_state_epoch_set_hash: row.target_state_epoch_set_hash.clone(),
        observation_window_started_at: row.observation_window_started_at,
        observation_window_completed_at: row.observation_window_completed_at,
        effective_valid_until: row.effective_valid_until,
    }
}

async fn seal_revision_authority_set_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    revision_id: Uuid,
) -> Result<RevisionAdjudicationAuthoritySetRefV1> {
    let rows = sqlx::query_as::<_, RevisionAuthorityRow>(
        r#"SELECT adjudication.organization_id,adjudication.hypothesis_revision_id,
                  wave.generation_seal_id,wave.generation_seal_hash,
                  plan.plan_id AS verification_plan_seal_id,
                  plan.plan_hash AS verification_plan_seal_hash,
                  plan.proof_path_set_hash,
                  plan.required_claim_component_set_hash AS claim_component_set_hash,
                  adjudication.revision_adjudication_id,
                  adjudication.adjudication_hash AS revision_adjudication_hash,
                  adjudication.outcome AS adjudication_outcome,
                  terminal.revision_terminal_decision_id,
                  terminal.decision_hash AS revision_terminal_decision_hash,
                  outcome_set.member_count AS latest_objective_outcome_member_count,
                  outcome_set.member_set_hash AS latest_objective_outcome_set_hash,
                  wave.wave_coverage_receipt_id,wave.wave_coverage_receipt_hash,
                  wave.coverage_membership_hash,wave.residual_membership_hash,
                  wave.consolidation_receipt_id,wave.consolidation_receipt_hash,
                  wave.fixed_point_receipt_id,wave.fixed_point_receipt_hash,
                  LEAST(adjudication.effective_valid_until,bundle.effective_valid_until)
                      AS authority_effective_valid_until,
                  bundle.organization_id AS bundle_organization_id,bundle.id,
                  bundle.relevant_root_count,bundle.relevant_root_set_hash,
                  bundle.member_count,bundle.member_set_hash,
                  bundle.semantic_authority_bundle_hash,
                  bundle.freshness_attestation_bundle_hash,
                  bundle.temporal_validity_bundle_hash,
                  bundle.temporal_validity_policy_set_hash,
                  bundle.target_state_epoch_set_hash,
                  bundle.observation_window_started_at,
                  bundle.observation_window_completed_at,bundle.effective_valid_until
             FROM hypothesis_revision_adjudications adjudication
             LEFT JOIN hypothesis_revision_terminal_decisions terminal
               ON terminal.revision_adjudication_id=adjudication.revision_adjudication_id
              AND terminal.hypothesis_revision_id=adjudication.hypothesis_revision_id
             JOIN attack_hypothesis_heads head
               ON head.operation_id=adjudication.operation_id
              AND head.organization_id=adjudication.organization_id
              AND ((adjudication.outcome='nonterminal'
                    AND head.head_revision_id=adjudication.hypothesis_revision_id
                    AND head.head_lifecycle_state='current'
                    AND head.head_epistemic_state NOT IN ('verified','refuted','invalid'))
                   OR (adjudication.outcome IN ('verified','refuted')
                       AND head.head_revision_id=terminal.terminal_successor_revision_id
                       AND head.head_lifecycle_state='closed'
                       AND head.head_epistemic_state=adjudication.outcome))
             JOIN attack_hypothesis_verification_plans plan
               ON plan.plan_id=adjudication.verification_plan_id
              AND plan.revision_id=adjudication.hypothesis_revision_id
             JOIN hypothesis_objective_outcome_set_seals outcome_set
               ON outcome_set.objective_outcome_set_seal_id=
                  adjudication.objective_outcome_set_seal_id
              AND outcome_set.sealed_at IS NOT NULL
             JOIN tool_truth_authority_bundle_seals bundle
               ON bundle.id=adjudication.tool_truth_authority_bundle_seal_id
              AND bundle.operation_id=adjudication.operation_id
              AND bundle.organization_id=adjudication.organization_id
              AND bundle.sealed_at IS NOT NULL
              AND bundle.relevant_root_count=3 AND bundle.member_count=3
              AND bundle.consistent_fresh_count=4 AND bundle.stale_or_invalid_count=0
             JOIN LATERAL (
                 SELECT generation_seal.seal_id AS generation_seal_id,
                        generation_seal.generation_hash AS generation_seal_hash,
                        coverage.wave_coverage_receipt_id,
                        coverage.receipt_hash AS wave_coverage_receipt_hash,
                        coverage.result_member_set_hash AS coverage_membership_hash,
                        COALESCE(fixed_point.residual_set_hash,
                                 consolidation.residual_set_hash) AS residual_membership_hash,
                        CASE WHEN fixed_point.fixed_point_receipt_id IS NULL
                             THEN consolidation.consolidation_receipt_id END
                             AS consolidation_receipt_id,
                        CASE WHEN fixed_point.fixed_point_receipt_id IS NULL
                             THEN consolidation.receipt_hash END
                             AS consolidation_receipt_hash,
                        fixed_point.fixed_point_receipt_id,
                        fixed_point.fixed_point_hash AS fixed_point_receipt_hash
                   FROM hypothesis_generation_members generation_member
                   JOIN hypothesis_generations generation
                     ON generation.generation_id=generation_member.generation_id
                   JOIN hypothesis_generation_seals generation_seal
                     ON generation_seal.generation_id=generation.generation_id
                   JOIN verification_wave_coverage_denominators denominator
                     ON denominator.generation_seal_id=generation_seal.seal_id
                    AND denominator.sealed_at IS NOT NULL
                   JOIN verification_wave_coverage_receipts coverage
                     ON coverage.wave_denominator_id=denominator.wave_denominator_id
                    AND coverage.coverage_status<>'invalid'
                   JOIN hypothesis_consolidation_batches batch
                     ON batch.generation_id=generation.generation_id
                    AND batch.wave_coverage_receipt_id=coverage.wave_coverage_receipt_id
                    AND batch.sealed_at IS NOT NULL
                   JOIN hypothesis_consolidation_receipts consolidation
                     ON consolidation.consolidation_batch_id=batch.consolidation_batch_id
                   LEFT JOIN hypothesis_fixed_point_receipts fixed_point
                     ON fixed_point.consolidation_receipt_id=
                        consolidation.consolidation_receipt_id
                    AND fixed_point.generation_id=generation.generation_id
                  WHERE generation_member.revision_id=adjudication.hypothesis_revision_id
                    AND generation.operation_id=adjudication.operation_id
                    AND generation.organization_id=adjudication.organization_id
                    AND NOT EXISTS (
                        SELECT 1
                          FROM verification_authority_quarantine_events quarantine
                          JOIN verification_campaign_coverage_receipts campaign_receipt
                            ON campaign_receipt.campaign_coverage_receipt_id=
                               quarantine.campaign_coverage_receipt_id
                          JOIN verification_campaign_coverage_denominators campaign_denominator
                            ON campaign_denominator.campaign_denominator_id=
                               campaign_receipt.campaign_denominator_id
                         WHERE campaign_denominator.wave_denominator_id=
                               denominator.wave_denominator_id
                    )
                    AND (
                        (consolidation.disposition='fixed_point'
                         AND fixed_point.fixed_point_receipt_id IS NOT NULL)
                        OR consolidation.disposition='blocked'
                    )
                    AND (adjudication.outcome<>'nonterminal'
                         OR fixed_point.fixed_point_receipt_id IS NOT NULL)
                    AND NOT EXISTS (
                        SELECT 1
                          FROM hypothesis_generation_members newer_member
                          JOIN hypothesis_generations newer
                            ON newer.generation_id=newer_member.generation_id
                         WHERE newer_member.revision_id=
                               adjudication.hypothesis_revision_id
                           AND newer.operation_id=adjudication.operation_id
                           AND newer.organization_id=adjudication.organization_id
                           AND (newer.generation_ordinal>generation.generation_ordinal
                                OR (newer.generation_ordinal=
                                    generation.generation_ordinal
                                    AND newer.generation_id>generation.generation_id))
                    )
                  ORDER BY generation.generation_ordinal DESC,generation.generation_id DESC
                  LIMIT 1
             ) wave ON TRUE
            WHERE adjudication.operation_id=$1
              AND ((adjudication.outcome='nonterminal'
                    AND terminal.revision_terminal_decision_id IS NULL)
                   OR (adjudication.outcome IN ('verified','refuted')
                       AND terminal.decision=adjudication.outcome))
              AND transaction_timestamp()<=adjudication.effective_valid_until
              AND transaction_timestamp()<=bundle.effective_valid_until
              AND NOT EXISTS (
                  SELECT 1 FROM hypothesis_revision_adjudications newer_adjudication
                   WHERE newer_adjudication.hypothesis_revision_id=
                         adjudication.hypothesis_revision_id
                     AND newer_adjudication.operation_id=adjudication.operation_id
                     AND (newer_adjudication.created_at>adjudication.created_at
                          OR (newer_adjudication.created_at=adjudication.created_at
                              AND newer_adjudication.revision_adjudication_id>
                                  adjudication.revision_adjudication_id))
              )
              AND NOT EXISTS (
                  SELECT 1
                    FROM hypothesis_objective_outcome_set_members outcome_member
                    LEFT JOIN hypothesis_objective_outcome_heads outcome_head
                      ON outcome_head.verification_plan_id=
                         adjudication.verification_plan_id
                     AND outcome_head.verification_objective_id=
                         outcome_member.verification_objective_id
                   WHERE outcome_member.objective_outcome_set_seal_id=
                         adjudication.objective_outcome_set_seal_id
                     AND (outcome_head.current_outcome_id IS NULL
                          OR outcome_head.current_outcome_id IS DISTINCT FROM
                             outcome_member.selected_current_outcome_id
                          OR outcome_head.current_ordinal IS DISTINCT FROM
                             outcome_member.selected_current_ordinal)
              )
              AND NOT EXISTS (
                  SELECT 1
                    FROM hypothesis_objective_outcome_set_members outcome_member
                    JOIN verification_authority_quarantine_events quarantine
                      ON quarantine.objective_outcome_receipt_id=
                         outcome_member.selected_current_outcome_id
                     AND quarantine.operation_id=adjudication.operation_id
                   WHERE outcome_member.objective_outcome_set_seal_id=
                         adjudication.objective_outcome_set_seal_id
              )
            ORDER BY adjudication.organization_id,adjudication.hypothesis_revision_id,
                     adjudication.revision_adjudication_id
            FOR SHARE OF head,adjudication,plan,outcome_set,bundle"#,
    )
    .bind(operation_id)
    .fetch_all(&mut **tx)
    .await?;
    let expected_current_adjudication_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM attack_hypothesis_heads
            WHERE operation_id=$1 AND head_lifecycle_state IN ('current','closed')
              AND head_epistemic_state<>'invalid'"#,
    )
    .bind(operation_id)
    .fetch_one(&mut **tx)
    .await?;
    if rows.is_empty()
        || usize::try_from(expected_current_adjudication_count).ok() != Some(rows.len())
    {
        return Err(conflict("REPORT_INPUT_REVISION_AUTHORITY_EMPTY"));
    }
    let authority_set_id = Uuid::new_v5(&revision_id, b"report-revision-authority-set.v1");
    let mut members = Vec::with_capacity(rows.len());
    let mut member_hashes = Vec::with_capacity(rows.len());
    let mut coverage_hashes = Vec::with_capacity(rows.len());
    let mut residual_hashes = Vec::with_capacity(rows.len());
    let mut earliest = None::<DateTime<Utc>>;
    for row in &rows {
        let wave_terminal = match (
            row.consolidation_receipt_id,
            row.consolidation_receipt_hash.as_deref(),
            row.fixed_point_receipt_id,
            row.fixed_point_receipt_hash.as_deref(),
        ) {
            (Some(receipt_id), Some(receipt_hash), None, None) => {
                WaveTerminalReceiptRefV1::Consolidation {
                    receipt_id,
                    receipt_hash: decode_tagged_hash(receipt_hash)?,
                }
            }
            (None, None, Some(receipt_id), Some(receipt_hash)) => {
                WaveTerminalReceiptRefV1::FixedPoint {
                    receipt_id,
                    receipt_hash: decode_tagged_hash(receipt_hash)?,
                }
            }
            _ => return Err(conflict("REPORT_INPUT_WAVE_TERMINAL_INVALID")),
        };
        let coverage_membership_hash = decode_tagged_hash(&row.coverage_membership_hash)?;
        let residual_membership_hash = decode_tagged_hash(&row.residual_membership_hash)?;
        let adjudication_outcome = match row.adjudication_outcome.as_str() {
            "nonterminal" => RevisionAdjudicationOutcomeV1::Nonterminal,
            "verified" => RevisionAdjudicationOutcomeV1::Verified,
            "refuted" => RevisionAdjudicationOutcomeV1::Refuted,
            _ => return Err(conflict("REPORT_INPUT_ADJUDICATION_OUTCOME_INVALID")),
        };
        let revision_terminal_decision_hash = row
            .revision_terminal_decision_hash
            .as_deref()
            .map(decode_tagged_hash)
            .transpose()?;
        if (adjudication_outcome == RevisionAdjudicationOutcomeV1::Nonterminal)
            != row.revision_terminal_decision_id.is_none()
            || row.revision_terminal_decision_id.is_none()
                != revision_terminal_decision_hash.is_none()
        {
            return Err(conflict("REPORT_INPUT_ADJUDICATION_OUTCOME_INVALID"));
        }
        let mut member = RevisionAdjudicationAuthorityMemberV1 {
            organization_id: row.organization_id,
            hypothesis_revision_id: row.hypothesis_revision_id,
            adjudication_tool_truth_authority: tool_truth_ref(&revision_bundle_row(row))?,
            generation_seal_id: row.generation_seal_id,
            generation_seal_hash: decode_tagged_hash(&row.generation_seal_hash)?,
            verification_plan_seal_id: row.verification_plan_seal_id,
            verification_plan_seal_hash: decode_tagged_hash(&row.verification_plan_seal_hash)?,
            proof_path_set_hash: decode_tagged_hash(&row.proof_path_set_hash)?,
            claim_component_set_hash: decode_tagged_hash(&row.claim_component_set_hash)?,
            revision_adjudication_id: row.revision_adjudication_id,
            revision_adjudication_hash: decode_tagged_hash(&row.revision_adjudication_hash)?,
            adjudication_outcome,
            revision_terminal_decision_id: row.revision_terminal_decision_id,
            revision_terminal_decision_hash,
            latest_objective_outcome_member_count: u64::try_from(
                row.latest_objective_outcome_member_count,
            )
            .map_err(|_| conflict("REPORT_INPUT_OBJECTIVE_COUNT_INVALID"))?,
            latest_objective_outcome_set_hash: decode_tagged_hash(
                &row.latest_objective_outcome_set_hash,
            )?,
            wave_terminal,
            final_wave_coverage_receipt_id: row.wave_coverage_receipt_id,
            final_wave_coverage_receipt_hash: decode_tagged_hash(&row.wave_coverage_receipt_hash)?,
            coverage_membership_hash,
            residual_membership_hash,
            effective_valid_until: row.authority_effective_valid_until,
            member_hash: [0; 32],
        };
        member.member_hash = digest_serializable(&member)?;
        earliest = Some(
            earliest
                .map(|current| current.min(member.effective_valid_until))
                .unwrap_or(member.effective_valid_until),
        );
        member_hashes.push(member.member_hash);
        coverage_hashes.push(coverage_membership_hash);
        residual_hashes.push(residual_membership_hash);
        members.push((row, member));
    }
    let authority_set_hash = aggregate_hash(
        "report_revision_adjudication_authority_set.v1",
        &member_hashes,
    )?;
    let coverage_membership_hash =
        aggregate_hash("report_revision_coverage_membership.v1", &coverage_hashes)?;
    let residual_membership_hash =
        aggregate_hash("report_revision_residual_membership.v1", &residual_hashes)?;
    let earliest_effective_valid_until =
        earliest.ok_or_else(|| conflict("REPORT_INPUT_REVISION_AUTHORITY_EMPTY"))?;
    let member_count = i64::try_from(members.len())
        .map_err(|_| conflict("REPORT_INPUT_AUTHORITY_COUNT_INVALID"))?;
    sqlx::query(
        r#"INSERT INTO report_input_revision_adjudication_sets(
               authority_set_id,revision_id,operation_id,authority_member_count,
               authority_set_hash,coverage_membership_hash,residual_membership_hash,
               earliest_effective_valid_until
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(authority_set_id)
    .bind(revision_id)
    .bind(operation_id)
    .bind(member_count)
    .bind(authority_set_hash.as_slice())
    .bind(coverage_membership_hash.as_slice())
    .bind(residual_membership_hash.as_slice())
    .bind(earliest_effective_valid_until)
    .execute(&mut **tx)
    .await?;
    for (ordinal, (row, member)) in members.into_iter().enumerate() {
        let (consolidation_receipt_id, fixed_point_receipt_id) = match &member.wave_terminal {
            WaveTerminalReceiptRefV1::Consolidation { receipt_id, .. } => (Some(*receipt_id), None),
            WaveTerminalReceiptRefV1::FixedPoint { receipt_id, .. } => (None, Some(*receipt_id)),
        };
        sqlx::query(
            r#"INSERT INTO report_input_revision_adjudication_members(
                   authority_set_id,revision_id,operation_id,ordinal,organization_id,
                   hypothesis_revision_id,adjudication_tool_truth_bundle_id,generation_seal_id,
                   verification_plan_seal_id,revision_adjudication_id,
                   adjudication_outcome,revision_terminal_decision_id,
                   revision_terminal_decision_hash,final_wave_coverage_receipt_id,
                   consolidation_receipt_id,fixed_point_receipt_id,typed_member,
                   effective_valid_until,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)"#,
        )
        .bind(authority_set_id)
        .bind(revision_id)
        .bind(operation_id)
        .bind(i32::try_from(ordinal).map_err(|_| conflict("REPORT_INPUT_AUTHORITY_COUNT_INVALID"))?)
        .bind(member.organization_id)
        .bind(member.hypothesis_revision_id)
        .bind(row.id)
        .bind(member.generation_seal_id)
        .bind(member.verification_plan_seal_id)
        .bind(member.revision_adjudication_id)
        .bind(match member.adjudication_outcome {
            RevisionAdjudicationOutcomeV1::Nonterminal => "nonterminal",
            RevisionAdjudicationOutcomeV1::Verified => "verified",
            RevisionAdjudicationOutcomeV1::Refuted => "refuted",
        })
        .bind(member.revision_terminal_decision_id)
        .bind(
            member
                .revision_terminal_decision_hash
                .map(encode_tagged_hash),
        )
        .bind(member.final_wave_coverage_receipt_id)
        .bind(consolidation_receipt_id)
        .bind(fixed_point_receipt_id)
        .bind(serde_json::to_value(&member)?)
        .bind(member.effective_valid_until)
        .bind(member.member_hash.as_slice())
        .execute(&mut **tx)
        .await?;
    }
    Ok(RevisionAdjudicationAuthoritySetRefV1 {
        authority_set_id,
        authority_member_count: u64::try_from(member_count)
            .map_err(|_| conflict("REPORT_INPUT_AUTHORITY_COUNT_INVALID"))?,
        authority_set_hash,
        coverage_membership_hash,
        residual_membership_hash,
        earliest_effective_valid_until,
    })
}

#[derive(sqlx::FromRow)]
struct StoredRevisionAuthorityMemberRow {
    ordinal: i32,
    organization_id: Uuid,
    hypothesis_revision_id: Uuid,
    adjudication_tool_truth_bundle_id: Uuid,
    generation_seal_id: Uuid,
    verification_plan_seal_id: Uuid,
    revision_adjudication_id: Uuid,
    adjudication_outcome: String,
    revision_terminal_decision_id: Option<Uuid>,
    revision_terminal_decision_hash: Option<String>,
    final_wave_coverage_receipt_id: Uuid,
    consolidation_receipt_id: Option<Uuid>,
    fixed_point_receipt_id: Option<Uuid>,
    typed_member: Value,
    effective_valid_until: DateTime<Utc>,
    member_hash: Vec<u8>,
}

fn encode_tagged_hash(value: [u8; 32]) -> String {
    format!(
        "sha256:{}",
        value
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

async fn revision_member_is_current_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    member: &RevisionAdjudicationAuthorityMemberV1,
) -> Result<bool> {
    let (terminal_receipt_id, terminal_receipt_hash, terminal_disposition) =
        match &member.wave_terminal {
            WaveTerminalReceiptRefV1::Consolidation {
                receipt_id,
                receipt_hash,
            } => (*receipt_id, encode_tagged_hash(*receipt_hash), "blocked"),
            WaveTerminalReceiptRefV1::FixedPoint {
                receipt_id,
                receipt_hash,
            } => (
                *receipt_id,
                encode_tagged_hash(*receipt_hash),
                "fixed_point",
            ),
        };
    let adjudication_outcome = match member.adjudication_outcome {
        RevisionAdjudicationOutcomeV1::Nonterminal => "nonterminal",
        RevisionAdjudicationOutcomeV1::Verified => "verified",
        RevisionAdjudicationOutcomeV1::Refuted => "refuted",
    };
    let terminal_decision_hash = member
        .revision_terminal_decision_hash
        .map(encode_tagged_hash);
    let is_current = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM hypothesis_revision_adjudications adjudication
                 LEFT JOIN hypothesis_revision_terminal_decisions terminal
                   ON terminal.revision_adjudication_id=adjudication.revision_adjudication_id
                  AND terminal.hypothesis_revision_id=adjudication.hypothesis_revision_id
                 JOIN attack_hypothesis_heads head
                   ON head.operation_id=adjudication.operation_id
                  AND head.organization_id=adjudication.organization_id
                  AND ((adjudication.outcome='nonterminal'
                        AND head.head_revision_id=adjudication.hypothesis_revision_id
                        AND head.head_lifecycle_state='current'
                        AND head.head_epistemic_state NOT IN ('verified','refuted','invalid'))
                       OR (adjudication.outcome IN ('verified','refuted')
                           AND head.head_revision_id=terminal.terminal_successor_revision_id
                           AND head.head_lifecycle_state='closed'
                           AND head.head_epistemic_state=adjudication.outcome))
                 JOIN attack_hypothesis_verification_plans plan
                   ON plan.plan_id=adjudication.verification_plan_id
                  AND plan.revision_id=adjudication.hypothesis_revision_id
                 JOIN hypothesis_objective_outcome_set_seals outcome_set
                   ON outcome_set.objective_outcome_set_seal_id=
                      adjudication.objective_outcome_set_seal_id
                  AND outcome_set.sealed_at IS NOT NULL
                 JOIN tool_truth_authority_bundle_seals bundle
                   ON bundle.id=adjudication.tool_truth_authority_bundle_seal_id
                  AND bundle.operation_id=terminal.operation_id
                  AND bundle.organization_id=terminal.organization_id
                  AND bundle.sealed_at IS NOT NULL
                  AND bundle.relevant_root_count=4 AND bundle.member_count=4
                  AND bundle.consistent_fresh_count=4 AND bundle.stale_or_invalid_count=0
                 JOIN hypothesis_generation_seals generation_seal
                   ON generation_seal.seal_id=$6
                 JOIN hypothesis_generations generation
                   ON generation.generation_id=generation_seal.generation_id
                  AND generation.operation_id=adjudication.operation_id
                  AND generation.organization_id=adjudication.organization_id
                 JOIN hypothesis_generation_members generation_member
                   ON generation_member.generation_id=generation.generation_id
                  AND generation_member.revision_id=adjudication.hypothesis_revision_id
                 JOIN verification_wave_coverage_denominators denominator
                   ON denominator.generation_seal_id=generation_seal.seal_id
                  AND denominator.sealed_at IS NOT NULL
                 JOIN verification_wave_coverage_receipts coverage
                   ON coverage.wave_denominator_id=denominator.wave_denominator_id
                  AND coverage.wave_coverage_receipt_id=$7
                  AND coverage.coverage_status<>'invalid'
                 JOIN hypothesis_consolidation_batches batch
                   ON batch.generation_id=generation.generation_id
                  AND batch.wave_coverage_receipt_id=coverage.wave_coverage_receipt_id
                  AND batch.sealed_at IS NOT NULL
                 JOIN hypothesis_consolidation_receipts consolidation
                   ON consolidation.consolidation_batch_id=batch.consolidation_batch_id
                 LEFT JOIN hypothesis_fixed_point_receipts fixed_point
                   ON fixed_point.consolidation_receipt_id=
                      consolidation.consolidation_receipt_id
                  AND fixed_point.generation_id=generation.generation_id
                WHERE adjudication.operation_id=$1
                  AND adjudication.organization_id=$2
                  AND adjudication.hypothesis_revision_id=$3
                  AND adjudication.revision_adjudication_id=$4
                  AND plan.plan_id=$5 AND bundle.id=$8
                  AND adjudication.outcome=$9
                  AND terminal.revision_terminal_decision_id IS NOT DISTINCT FROM $10
                  AND transaction_timestamp()<=adjudication.effective_valid_until
                  AND transaction_timestamp()<=bundle.effective_valid_until
                  AND generation_seal.generation_hash=$11
                  AND plan.plan_hash=$12 AND plan.proof_path_set_hash=$13
                  AND plan.required_claim_component_set_hash=$14
                  AND adjudication.adjudication_hash=$15
                  AND terminal.decision_hash IS NOT DISTINCT FROM $16
                  AND outcome_set.member_count=$17 AND outcome_set.member_set_hash=$18
                  AND coverage.receipt_hash=$19
                  AND coverage.result_member_set_hash=$20
                  AND COALESCE(fixed_point.residual_set_hash,
                               consolidation.residual_set_hash)=$21
                  AND consolidation.disposition=$22
                  AND (($22='blocked'
                        AND consolidation.consolidation_receipt_id=$23
                        AND consolidation.receipt_hash=$24
                        AND fixed_point.fixed_point_receipt_id IS NULL)
                       OR ($22='fixed_point'
                           AND fixed_point.fixed_point_receipt_id=$23
                           AND fixed_point.fixed_point_hash=$24))
                  AND ($9<>'nonterminal'
                       OR fixed_point.fixed_point_receipt_id IS NOT NULL)
                  AND NOT EXISTS (
                      SELECT 1 FROM hypothesis_revision_adjudications newer_adjudication
                       WHERE newer_adjudication.hypothesis_revision_id=
                             adjudication.hypothesis_revision_id
                         AND newer_adjudication.operation_id=adjudication.operation_id
                         AND (newer_adjudication.created_at>adjudication.created_at
                              OR (newer_adjudication.created_at=adjudication.created_at
                                  AND newer_adjudication.revision_adjudication_id>
                                      adjudication.revision_adjudication_id))
                  )
                  AND NOT EXISTS (
                      SELECT 1
                        FROM hypothesis_generation_members newer_member
                        JOIN hypothesis_generations newer
                          ON newer.generation_id=newer_member.generation_id
                       WHERE newer_member.revision_id=terminal.hypothesis_revision_id
                         AND newer.operation_id=terminal.operation_id
                         AND newer.organization_id=terminal.organization_id
                         AND (newer.generation_ordinal>generation.generation_ordinal
                              OR (newer.generation_ordinal=generation.generation_ordinal
                                  AND newer.generation_id>generation.generation_id))
                  )
                  AND NOT EXISTS (
                      SELECT 1
                        FROM hypothesis_objective_outcome_set_members outcome_member
                        JOIN verification_authority_quarantine_events quarantine
                          ON quarantine.objective_outcome_receipt_id=
                             outcome_member.selected_current_outcome_id
                         AND quarantine.operation_id=terminal.operation_id
                       WHERE outcome_member.objective_outcome_set_seal_id=
                             adjudication.objective_outcome_set_seal_id
                  )
                  AND NOT EXISTS (
                      SELECT 1
                        FROM verification_authority_quarantine_events quarantine
                        JOIN verification_campaign_coverage_receipts campaign_receipt
                          ON campaign_receipt.campaign_coverage_receipt_id=
                             quarantine.campaign_coverage_receipt_id
                        JOIN verification_campaign_coverage_denominators campaign_denominator
                          ON campaign_denominator.campaign_denominator_id=
                             campaign_receipt.campaign_denominator_id
                       WHERE campaign_denominator.wave_denominator_id=
                             denominator.wave_denominator_id
                  )
           )"#,
    )
    .bind(operation_id)
    .bind(member.organization_id)
    .bind(member.hypothesis_revision_id)
    .bind(member.revision_adjudication_id)
    .bind(member.verification_plan_seal_id)
    .bind(member.generation_seal_id)
    .bind(member.final_wave_coverage_receipt_id)
    .bind(member.adjudication_tool_truth_authority.bundle_id)
    .bind(adjudication_outcome)
    .bind(member.revision_terminal_decision_id)
    .bind(encode_tagged_hash(member.generation_seal_hash))
    .bind(encode_tagged_hash(member.verification_plan_seal_hash))
    .bind(encode_tagged_hash(member.proof_path_set_hash))
    .bind(encode_tagged_hash(member.claim_component_set_hash))
    .bind(encode_tagged_hash(member.revision_adjudication_hash))
    .bind(terminal_decision_hash)
    .bind(
        i64::try_from(member.latest_objective_outcome_member_count)
            .map_err(|_| conflict("REPORT_INPUT_OBJECTIVE_COUNT_INVALID"))?,
    )
    .bind(encode_tagged_hash(member.latest_objective_outcome_set_hash))
    .bind(encode_tagged_hash(member.final_wave_coverage_receipt_hash))
    .bind(encode_tagged_hash(member.coverage_membership_hash))
    .bind(encode_tagged_hash(member.residual_membership_hash))
    .bind(terminal_disposition)
    .bind(terminal_receipt_id)
    .bind(terminal_receipt_hash)
    .fetch_one(&mut *connection)
    .await?;
    Ok(is_current)
}

async fn validate_revision_authority_set_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    revision_id: Uuid,
    authority: &RevisionAdjudicationAuthoritySetRefV1,
    observed_at: DateTime<Utc>,
) -> Result<()> {
    #[derive(sqlx::FromRow)]
    struct RevisionAuthoritySetHeaderRow {
        authority_member_count: i64,
        authority_set_hash: Vec<u8>,
        coverage_membership_hash: Vec<u8>,
        residual_membership_hash: Vec<u8>,
        earliest_effective_valid_until: DateTime<Utc>,
    }

    let header = sqlx::query_as::<_, RevisionAuthoritySetHeaderRow>(
        r#"SELECT authority_member_count,authority_set_hash,coverage_membership_hash,
                  residual_membership_hash,earliest_effective_valid_until
             FROM report_input_revision_adjudication_sets
            WHERE authority_set_id=$1 AND revision_id=$2 AND operation_id=$3 FOR SHARE"#,
    )
    .bind(authority.authority_set_id)
    .bind(revision_id)
    .bind(operation_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(header) = header else {
        return Err(conflict("REPORT_INPUT_REVISION_AUTHORITY_SET_MISSING"));
    };
    if u64::try_from(header.authority_member_count).ok() != Some(authority.authority_member_count)
        || header.authority_set_hash.as_slice() != authority.authority_set_hash
        || header.coverage_membership_hash.as_slice() != authority.coverage_membership_hash
        || header.residual_membership_hash.as_slice() != authority.residual_membership_hash
        || header.earliest_effective_valid_until != authority.earliest_effective_valid_until
        || header.earliest_effective_valid_until <= observed_at
    {
        return Err(conflict("REPORT_INPUT_REVISION_AUTHORITY_SET_STALE"));
    }
    let current_adjudication_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM attack_hypothesis_heads
            WHERE operation_id=$1 AND head_lifecycle_state IN ('current','closed')
              AND head_epistemic_state<>'invalid'"#,
    )
    .bind(operation_id)
    .fetch_one(&mut *connection)
    .await?;
    let members = sqlx::query_as::<_, StoredRevisionAuthorityMemberRow>(
        r#"SELECT ordinal,organization_id,hypothesis_revision_id,
                  adjudication_tool_truth_bundle_id,generation_seal_id,
                  verification_plan_seal_id,revision_adjudication_id,
                  revision_terminal_decision_id,final_wave_coverage_receipt_id,
                  consolidation_receipt_id,fixed_point_receipt_id,typed_member,
                  effective_valid_until,member_hash
             FROM report_input_revision_adjudication_members
            WHERE authority_set_id=$1 ORDER BY ordinal FOR SHARE"#,
    )
    .bind(authority.authority_set_id)
    .fetch_all(&mut *connection)
    .await?;
    if members.len() != usize::try_from(header.authority_member_count).unwrap_or(usize::MAX)
        || i64::try_from(members.len()).ok() != Some(current_adjudication_count)
    {
        return Err(conflict("REPORT_INPUT_REVISION_AUTHORITY_SET_STALE"));
    }
    let mut hashes = Vec::with_capacity(members.len());
    let mut coverage_hashes = Vec::with_capacity(members.len());
    let mut residual_hashes = Vec::with_capacity(members.len());
    let mut earliest = None::<DateTime<Utc>>;
    for (expected_ordinal, stored) in members.iter().enumerate() {
        if usize::try_from(stored.ordinal).ok() != Some(expected_ordinal)
            || stored.effective_valid_until <= observed_at
        {
            return Err(conflict("REPORT_INPUT_REVISION_AUTHORITY_MEMBER_STALE"));
        }
        let member: RevisionAdjudicationAuthorityMemberV1 =
            serde_json::from_value(stored.typed_member.clone())
                .map_err(|_| conflict("REPORT_INPUT_REVISION_AUTHORITY_MEMBER_CORRUPT"))?;
        let (consolidation_receipt_id, fixed_point_receipt_id) = match &member.wave_terminal {
            WaveTerminalReceiptRefV1::Consolidation { receipt_id, .. } => (Some(*receipt_id), None),
            WaveTerminalReceiptRefV1::FixedPoint { receipt_id, .. } => (None, Some(*receipt_id)),
        };
        let stored_outcome = match member.adjudication_outcome {
            RevisionAdjudicationOutcomeV1::Nonterminal => "nonterminal",
            RevisionAdjudicationOutcomeV1::Verified => "verified",
            RevisionAdjudicationOutcomeV1::Refuted => "refuted",
        };
        let terminal_decision_hash = member
            .revision_terminal_decision_hash
            .map(encode_tagged_hash);
        let nonterminal_shape_valid = member.adjudication_outcome
            != RevisionAdjudicationOutcomeV1::Nonterminal
            || (member.revision_terminal_decision_id.is_none()
                && terminal_decision_hash.is_none()
                && fixed_point_receipt_id.is_some());
        let mut hash_material = member.clone();
        hash_material.member_hash = [0; 32];
        let recomputed_hash = digest_serializable(&hash_material)?;
        let live_adjudication_bundle = sqlx::query_as::<_, ToolTruthBundleRow>(
            r#"SELECT organization_id,id,relevant_root_count,relevant_root_set_hash,
                      member_count,member_set_hash,semantic_authority_bundle_hash,
                      freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
                      temporal_validity_policy_set_hash,target_state_epoch_set_hash,
                      observation_window_started_at,observation_window_completed_at,
                      effective_valid_until
                 FROM tool_truth_authority_bundle_seals
                WHERE id=$1 AND operation_id=$2 AND organization_id=$3
                  AND sealed_at IS NOT NULL AND relevant_root_count=3 AND member_count=3
                  AND consistent_fresh_count=4 AND stale_or_invalid_count=0
                  AND transaction_timestamp()<=effective_valid_until
                FOR SHARE"#,
        )
        .bind(stored.adjudication_tool_truth_bundle_id)
        .bind(operation_id)
        .bind(stored.organization_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or_else(|| conflict("REPORT_INPUT_REVISION_AUTHORITY_MEMBER_STALE"))?;
        if !tool_truth_bundle_members_are_live_on(
            connection,
            live_adjudication_bundle.id,
            operation_id,
            stored.organization_id,
        )
        .await?
        {
            return Err(conflict("REPORT_INPUT_REVISION_AUTHORITY_MEMBER_STALE"));
        }
        let live_adjudication_authority = tool_truth_ref(&live_adjudication_bundle)?;
        let adjudication_valid_until: DateTime<Utc> = sqlx::query_scalar(
            r#"SELECT effective_valid_until
                 FROM hypothesis_revision_adjudications
                WHERE revision_adjudication_id=$1 AND operation_id=$2
                  AND organization_id=$3 FOR SHARE"#,
        )
        .bind(stored.revision_adjudication_id)
        .bind(operation_id)
        .bind(stored.organization_id)
        .fetch_one(&mut *connection)
        .await?;
        let live_effective_valid_until =
            adjudication_valid_until.min(live_adjudication_bundle.effective_valid_until);
        if member.organization_id != stored.organization_id
            || member.hypothesis_revision_id != stored.hypothesis_revision_id
            || member.adjudication_tool_truth_authority.bundle_id
                != stored.adjudication_tool_truth_bundle_id
            || member.adjudication_tool_truth_authority != live_adjudication_authority
            || member.generation_seal_id != stored.generation_seal_id
            || member.verification_plan_seal_id != stored.verification_plan_seal_id
            || member.revision_adjudication_id != stored.revision_adjudication_id
            || stored_outcome != stored.adjudication_outcome
            || member.revision_terminal_decision_id != stored.revision_terminal_decision_id
            || terminal_decision_hash != stored.revision_terminal_decision_hash
            || !nonterminal_shape_valid
            || member.final_wave_coverage_receipt_id != stored.final_wave_coverage_receipt_id
            || consolidation_receipt_id != stored.consolidation_receipt_id
            || fixed_point_receipt_id != stored.fixed_point_receipt_id
            || member.effective_valid_until != stored.effective_valid_until
            || member.effective_valid_until != live_effective_valid_until
            || member.member_hash != recomputed_hash
            || stored.member_hash.as_slice() != recomputed_hash
            || !revision_member_is_current_on(connection, operation_id, &member).await?
        {
            return Err(conflict("REPORT_INPUT_REVISION_AUTHORITY_MEMBER_STALE"));
        }
        hashes.push(recomputed_hash);
        coverage_hashes.push(member.coverage_membership_hash);
        residual_hashes.push(member.residual_membership_hash);
        earliest = Some(
            earliest
                .map(|current| current.min(member.effective_valid_until))
                .unwrap_or(member.effective_valid_until),
        );
    }
    if aggregate_hash("report_revision_adjudication_authority_set.v1", &hashes)?
        != authority.authority_set_hash
        || aggregate_hash("report_revision_coverage_membership.v1", &coverage_hashes)?
            != authority.coverage_membership_hash
        || aggregate_hash("report_revision_residual_membership.v1", &residual_hashes)?
            != authority.residual_membership_hash
        || earliest != Some(authority.earliest_effective_valid_until)
    {
        return Err(conflict("REPORT_INPUT_REVISION_AUTHORITY_SET_CORRUPT"));
    }
    Ok(())
}

pub async fn validate_current_report_input_authority_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    revision_id: Uuid,
    seal: &ReportInputSealV1,
    observed_at: DateTime<Utc>,
) -> Result<()> {
    let tool_truth_set = match seal {
        ReportInputSealV1::RevisionAdjudication(value) => &value.report_tool_truth_authority_set,
        ReportInputSealV1::Legacy(value) => &value.report_tool_truth_authority_set,
    };
    validate_report_tool_truth_set_on(
        connection,
        operation_id,
        revision_id,
        tool_truth_set,
        observed_at,
    )
    .await?;
    match seal {
        ReportInputSealV1::RevisionAdjudication(value) => {
            validate_revision_authority_set_on(
                connection,
                operation_id,
                revision_id,
                &value.revision_adjudication_authority_set,
                observed_at,
            )
            .await
        }
        ReportInputSealV1::Legacy(value) => {
            let latest: Option<(Uuid, String, String)> = sqlx::query_as(
                r#"SELECT seal_id,seal_hash,limitation_membership_hash
                     FROM legacy_report_authority_seals
                    WHERE operation_id=$1 ORDER BY sealed_at DESC,seal_id DESC LIMIT 1
                    FOR SHARE"#,
            )
            .bind(operation_id)
            .fetch_optional(&mut *connection)
            .await?;
            let expected = (
                value.legacy_report_authority_seal_id,
                encode_tagged_hash(value.legacy_report_authority_seal_hash),
                encode_tagged_hash(value.limitation_membership_hash),
            );
            if latest.as_ref() != Some(&expected) {
                return Err(conflict("REPORT_INPUT_LEGACY_AUTHORITY_STALE"));
            }
            Ok(())
        }
    }
}

/// Revalidates a persisted report revision at every reuse/read seam.
/// Known authority drift is returned as a stable REPORT_INPUT_* rejection;
/// infrastructure failures remain ordinary database errors.
pub async fn validate_persisted_report_input_authority_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    revision_id: Uuid,
) -> Result<()> {
    let stored = sqlx::query_as::<_, PersistedReportInputAuthorityRow>(
        r#"SELECT seal.typed_seal,open.authority_contract,
                  seal.tool_truth_authority_set_id,
                  seal.revision_adjudication_authority_set_id,
                  seal.legacy_report_authority_seal_id,
                  seal.source_member_count,seal.source_set_hash,seal.report_input_hash,
                  seal.effective_valid_until,
                  revision.source_set_hash AS revision_source_set_hash,
                  (SELECT COUNT(*) FROM report_source_manifest source_count
                    WHERE source_count.revision_id=seal.revision_id) AS manifest_count,
                  transaction_timestamp() AS observed_at,
                  EXISTS(
                      SELECT 1
                        FROM report_authority_invalidation_events invalidation
                       WHERE invalidation.report_revision_id=seal.revision_id
                         AND invalidation.operation_id=seal.operation_id
                  ) AS invalidated
             FROM report_input_seals seal
             JOIN report_input_open_headers open ON open.open_id=seal.open_id
             JOIN report_revisions revision ON revision.revision_id=seal.revision_id
            JOIN reports report ON report.report_id=revision.report_id
            WHERE seal.revision_id=$1 AND seal.operation_id=$2
              AND report.operation_id=$2
            FOR SHARE OF seal,open,revision"#,
    )
    .bind(revision_id)
    .bind(operation_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(PersistedReportInputAuthorityRow {
        typed_seal,
        authority_contract,
        tool_truth_authority_set_id,
        revision_adjudication_authority_set_id,
        legacy_report_authority_seal_id,
        source_member_count,
        source_set_hash: stored_source_set_hash,
        report_input_hash: stored_report_input_hash,
        effective_valid_until: stored_effective_valid_until,
        revision_source_set_hash,
        manifest_count,
        observed_at,
        invalidated,
    }) = stored
    else {
        return Err(conflict("REPORT_INPUT_SEAL_MISSING"));
    };
    if invalidated {
        return Err(conflict("REPORT_INPUT_AUTHORITY_INVALIDATED"));
    }
    let typed: ReportInputSealV1 =
        serde_json::from_value(typed_seal).map_err(|_| conflict("REPORT_INPUT_SEAL_CORRUPT"))?;
    let source_set_hash = decode_bare_hash(&revision_source_set_hash)?;
    let (identity_matches, typed_effective_valid_until) = match &typed {
        ReportInputSealV1::RevisionAdjudication(value) => (
            authority_contract == "revision_adjudication"
                && tool_truth_authority_set_id
                    == value.report_tool_truth_authority_set.authority_set_id
                && revision_adjudication_authority_set_id
                    == Some(value.revision_adjudication_authority_set.authority_set_id)
                && legacy_report_authority_seal_id.is_none(),
            value
                .report_tool_truth_authority_set
                .earliest_effective_valid_until
                .min(
                    value
                        .revision_adjudication_authority_set
                        .earliest_effective_valid_until,
                ),
        ),
        ReportInputSealV1::Legacy(value) => (
            authority_contract == "legacy"
                && tool_truth_authority_set_id
                    == value.report_tool_truth_authority_set.authority_set_id
                && revision_adjudication_authority_set_id.is_none()
                && legacy_report_authority_seal_id == Some(value.legacy_report_authority_seal_id),
            value
                .report_tool_truth_authority_set
                .earliest_effective_valid_until,
        ),
    };
    if !identity_matches
        || i64::try_from(typed.source_member_count()).ok() != Some(source_member_count)
        || manifest_count != source_member_count
        || stored_source_set_hash.as_slice() != source_set_hash
        || stored_report_input_hash.as_slice() != typed.report_input_hash()
        || stored_effective_valid_until != typed_effective_valid_until
        || stored_effective_valid_until <= observed_at
    {
        return Err(conflict("REPORT_INPUT_SEAL_STALE"));
    }
    typed
        .validate(
            usize::try_from(manifest_count)
                .map_err(|_| conflict("REPORT_INPUT_SOURCE_COUNT_INVALID"))?,
            source_set_hash,
            observed_at,
        )
        .map_err(|_| conflict("REPORT_INPUT_SEAL_STALE"))?;
    validate_current_report_input_authority_on(
        connection,
        operation_id,
        revision_id,
        &typed,
        observed_at,
    )
    .await
}

/// Compatibility name used by historical artifact reads. The assertion is
/// intentionally the same fail-closed revalidation performed at every live
/// report read seam; historical callers decide whether a stale authority is
/// displayable as historical context.
pub async fn assert_current_report_input_authority_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    revision_id: Uuid,
) -> Result<()> {
    validate_persisted_report_input_authority_on(connection, operation_id, revision_id).await
}

pub fn is_report_input_authority_rejection(error: &crate::DbError) -> bool {
    error.to_string().contains("REPORT_INPUT_")
}

pub async fn seal_current_report_input_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    revision_id: Uuid,
    snapshot: &ReportSourceSnapshot,
) -> Result<ReportInputSealV1> {
    let rollout_mode: String = sqlx::query_scalar(
        "SELECT investigation_rollout_mode FROM operation_state WHERE operation_id=$1 FOR SHARE",
    )
    .bind(operation_id)
    .fetch_one(&mut **tx)
    .await?;
    let report_tool_truth_authority_set =
        seal_report_tool_truth_set_on(tx, operation_id, revision_id).await?;
    let source_member_count = u64::try_from(snapshot.ordered_sources.len())
        .map_err(|_| conflict("REPORT_INPUT_SOURCE_COUNT_INVALID"))?;
    let mut seal = if matches!(
        rollout_mode.as_str(),
        "registry_authoritative_legacy_projection" | "new_only"
    ) {
        ReportInputSealV1::RevisionAdjudication(RevisionAdjudicationReportInputSealV1 {
            report_tool_truth_authority_set,
            revision_adjudication_authority_set: seal_revision_authority_set_on(
                tx,
                operation_id,
                revision_id,
            )
            .await?,
            source_member_count,
            source_set_hash: snapshot.source_set_hash,
            report_input_hash: [0; 32],
        })
    } else {
        let legacy: (Uuid, String, String) = sqlx::query_as(
            r#"SELECT seal_id,seal_hash,limitation_membership_hash
                 FROM legacy_report_authority_seals
                WHERE operation_id=$1 ORDER BY sealed_at DESC,seal_id DESC LIMIT 1
                FOR SHARE"#,
        )
        .bind(operation_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict("REPORT_INPUT_LEGACY_AUTHORITY_MISSING"))?;
        ReportInputSealV1::Legacy(LegacyReportInputSealV1 {
            report_tool_truth_authority_set,
            legacy_report_authority_seal_id: legacy.0,
            legacy_report_authority_seal_hash: decode_tagged_hash(&legacy.1)?,
            final_scope_source_set_hash: snapshot.source_set_hash,
            source_member_count,
            source_set_hash: snapshot.source_set_hash,
            limitation_membership_hash: decode_tagged_hash(&legacy.2)?,
            mandatory_limitation_code: LegacyCoverageLimitationCode::LegacyCoverageUnavailable,
            report_input_hash: [0; 32],
        })
    };
    let report_input_hash = seal.compute_report_input_hash()?;
    match &mut seal {
        ReportInputSealV1::RevisionAdjudication(value) => {
            value.report_input_hash = report_input_hash
        }
        ReportInputSealV1::Legacy(value) => value.report_input_hash = report_input_hash,
    }
    Ok(seal)
}
