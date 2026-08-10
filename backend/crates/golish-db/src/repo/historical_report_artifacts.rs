//! Controlled metadata/read-attestation boundary for retained report artifacts.
//!
//! The filesystem reader consumes the server-owned preparation and must use
//! the hardened content-addressed artifact read.  This repository then binds
//! the bytes actually observed to an append-only, authority-time attestation.

use chrono::{DateTime, Utc};
use golish_reporting_domain::{HistoricalArtifactReadAuthorityV0, HistoricalAuthorityTimeStatusV0};
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{DbError, Result};

fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

fn tagged_hash(value: &Value) -> String {
    format!(
        "sha256:{}",
        super::operation_scope_decisions::sha256_json(value)
    )
}

fn valid_tagged_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn decode_tagged_hash(value: &str) -> Result<[u8; 32]> {
    if !valid_tagged_hash(value) {
        return Err(conflict("HISTORICAL_ARTIFACT_HASH_INVALID"));
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes()[7..].chunks_exact(2).enumerate() {
        let high = (pair[0] as char)
            .to_digit(16)
            .ok_or_else(|| conflict("HISTORICAL_ARTIFACT_HASH_INVALID"))?;
        let low = (pair[1] as char)
            .to_digit(16)
            .ok_or_else(|| conflict("HISTORICAL_ARTIFACT_HASH_INVALID"))?;
        bytes[index] = u8::try_from((high << 4) | low)
            .map_err(|_| conflict("HISTORICAL_ARTIFACT_HASH_INVALID"))?;
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateHistoricalArtifactReceiptV0 {
    pub report_id: Uuid,
    pub revision_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub artifact_kind: String,
    pub content_key: String,
    pub sha256: String,
    pub storage_path: String,
    pub byte_len: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricalArtifactReadPreparationV0 {
    pub receipt_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub artifact_kind: String,
    pub content_key: String,
    pub sha256: String,
    pub storage_path: String,
    pub byte_len: i64,
    pub metadata_manifest_hash: String,
    pub authority_time_status: HistoricalAuthorityTimeStatusV0,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttestHistoricalArtifactReadV0 {
    pub receipt_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub principal_id: Uuid,
    pub request_private_snapshot_hash: String,
    pub observed_sha256: String,
    pub observed_byte_len: i64,
}

#[derive(sqlx::FromRow)]
struct HistoricalArtifactRow {
    receipt_id: Uuid,
    revision_id: Uuid,
    operation_id: Uuid,
    project_scope_id: Uuid,
    artifact_kind: String,
    content_key: String,
    sha256: String,
    storage_path: String,
    byte_len: i64,
    metadata_manifest_hash: String,
    publication_status: String,
    input_effective_valid_until: Option<DateTime<Utc>>,
    observed_at: DateTime<Utc>,
    input_authority_live: bool,
    invalidated: bool,
}

fn authority_time_status(row: &HistoricalArtifactRow) -> HistoricalAuthorityTimeStatusV0 {
    if row.invalidated {
        HistoricalAuthorityTimeStatusV0::RevokedHistory
    } else if row.publication_status == "final"
        && row.input_authority_live
        && row
            .input_effective_valid_until
            .is_some_and(|valid_until| valid_until > row.observed_at)
    {
        HistoricalAuthorityTimeStatusV0::AsOfFresh
    } else {
        HistoricalAuthorityTimeStatusV0::TemporallyStale
    }
}

fn authority_time_status_wire(status: HistoricalAuthorityTimeStatusV0) -> &'static str {
    match status {
        HistoricalAuthorityTimeStatusV0::AsOfFresh => "as_of_fresh",
        HistoricalAuthorityTimeStatusV0::TemporallyStale => "temporally_stale",
        HistoricalAuthorityTimeStatusV0::RevokedHistory => "revoked_history",
    }
}

pub async fn create_historical_artifact_receipt_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &CreateHistoricalArtifactReceiptV0,
) -> Result<Uuid> {
    if input.artifact_kind.trim().is_empty()
        || input.content_key.trim().is_empty()
        || input.storage_path.trim().is_empty()
        || input.byte_len < 0
        || input.sha256.len() != 64
        || !input
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(conflict("HISTORICAL_ARTIFACT_METADATA_INVALID"));
    }
    let receipt_id = Uuid::new_v5(
        &input.revision_id,
        format!("historical-artifact.v0:{}", input.artifact_kind).as_bytes(),
    );
    let metadata_manifest_hash = tagged_hash(&json!({
        "schema":"historical_report_artifact_receipt.v0",
        "receipt_id":receipt_id,
        "report_id":input.report_id,
        "revision_id":input.revision_id,
        "operation_id":input.operation_id,
        "project_scope_id":input.project_scope_id,
        "artifact_kind":input.artifact_kind,
        "content_key":input.content_key,
        "sha256":input.sha256,
        "storage_path":input.storage_path,
        "byte_len":input.byte_len,
    }));
    sqlx::query(
        r#"INSERT INTO historical_report_artifact_receipts(
               receipt_id,report_id,revision_id,operation_id,project_scope_id,
               artifact_kind,content_key,sha256,storage_path,byte_len,metadata_manifest_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(receipt_id)
    .bind(input.report_id)
    .bind(input.revision_id)
    .bind(input.operation_id)
    .bind(input.project_scope_id)
    .bind(&input.artifact_kind)
    .bind(&input.content_key)
    .bind(&input.sha256)
    .bind(&input.storage_path)
    .bind(input.byte_len)
    .bind(&metadata_manifest_hash)
    .execute(&mut **tx)
    .await?;
    Ok(receipt_id)
}

async fn locked_receipt_on(
    tx: &mut Transaction<'_, Postgres>,
    receipt_id: Uuid,
    operation_id: Uuid,
    project_scope_id: Uuid,
    principal_id: Uuid,
) -> Result<HistoricalArtifactRow> {
    let mut row = sqlx::query_as::<_, HistoricalArtifactRow>(
        r#"SELECT artifact.receipt_id,artifact.revision_id,
                  artifact.operation_id,artifact.project_scope_id,
                  artifact.artifact_kind,artifact.content_key,artifact.sha256,
                  artifact.storage_path,artifact.byte_len,artifact.metadata_manifest_hash,
                  revision.publication_status,input_seal.effective_valid_until
                      AS input_effective_valid_until,
                  transaction_timestamp() AS observed_at,
                  FALSE AS input_authority_live,
                  EXISTS(
                      SELECT 1 FROM report_authority_invalidation_events invalidation
                     WHERE invalidation.report_revision_id=artifact.revision_id
                       AND invalidation.operation_id=artifact.operation_id
                  ) OR EXISTS(
                      SELECT 1
                        FROM report_input_revision_adjudication_members authority_member
                        JOIN hypothesis_revision_adjudications adjudication
                          ON adjudication.revision_adjudication_id=
                             authority_member.revision_adjudication_id
                        JOIN hypothesis_objective_outcome_set_members outcome_member
                          ON outcome_member.objective_outcome_set_seal_id=
                             adjudication.objective_outcome_set_seal_id
                        JOIN verification_authority_quarantine_events quarantine
                          ON quarantine.objective_outcome_receipt_id=
                             outcome_member.selected_current_outcome_id
                         AND quarantine.operation_id=artifact.operation_id
                       WHERE authority_member.revision_id=artifact.revision_id
                  ) OR EXISTS(
                      SELECT 1
                        FROM report_input_revision_adjudication_members authority_member
                        JOIN verification_wave_coverage_receipts wave_receipt
                          ON wave_receipt.wave_coverage_receipt_id=
                             authority_member.final_wave_coverage_receipt_id
                        JOIN verification_campaign_coverage_denominators campaign_denominator
                          ON campaign_denominator.wave_denominator_id=
                             wave_receipt.wave_denominator_id
                        JOIN verification_campaign_coverage_receipts campaign_receipt
                          ON campaign_receipt.campaign_denominator_id=
                             campaign_denominator.campaign_denominator_id
                        JOIN verification_authority_quarantine_events quarantine
                          ON quarantine.campaign_coverage_receipt_id=
                             campaign_receipt.campaign_coverage_receipt_id
                         AND quarantine.operation_id=artifact.operation_id
                       WHERE authority_member.revision_id=artifact.revision_id
                  ) AS invalidated
             FROM historical_report_artifact_receipts artifact
             JOIN report_revisions revision ON revision.revision_id=artifact.revision_id
             LEFT JOIN report_input_seals input_seal
               ON input_seal.revision_id=artifact.revision_id
             JOIN operator_principals principal ON principal.id=$4
            WHERE artifact.receipt_id=$1 AND artifact.operation_id=$2
              AND artifact.project_scope_id=$3
              AND principal.active AND principal.principal_kind='local_operator'
            FOR SHARE OF artifact,revision,principal"#,
    )
    .bind(receipt_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(principal_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict("HISTORICAL_ARTIFACT_READ_FORBIDDEN"))?;
    if row.invalidated {
        return Err(conflict("HISTORICAL_ARTIFACT_AUTHORITY_REVOKED"));
    }
    row.input_authority_live =
        match super::report_input_authority::assert_current_report_input_authority_on(
            tx,
            row.operation_id,
            row.revision_id,
        )
        .await
        {
            Ok(()) => true,
            Err(error)
                if super::report_input_authority::is_report_input_authority_rejection(&error) =>
            {
                false
            }
            Err(error) => return Err(error),
        };
    Ok(row)
}

pub async fn prepare_historical_artifact_read_on(
    tx: &mut Transaction<'_, Postgres>,
    receipt_id: Uuid,
    operation_id: Uuid,
    project_scope_id: Uuid,
    principal_id: Uuid,
) -> Result<HistoricalArtifactReadPreparationV0> {
    let row =
        locked_receipt_on(tx, receipt_id, operation_id, project_scope_id, principal_id).await?;
    let authority_time_status = authority_time_status(&row);
    Ok(HistoricalArtifactReadPreparationV0 {
        receipt_id: row.receipt_id,
        operation_id: row.operation_id,
        project_scope_id: row.project_scope_id,
        artifact_kind: row.artifact_kind,
        content_key: row.content_key,
        sha256: row.sha256,
        storage_path: row.storage_path,
        byte_len: row.byte_len,
        metadata_manifest_hash: row.metadata_manifest_hash,
        authority_time_status,
    })
}

pub async fn attest_historical_artifact_read_on(
    tx: &mut Transaction<'_, Postgres>,
    input: AttestHistoricalArtifactReadV0,
) -> Result<HistoricalArtifactReadAuthorityV0> {
    if !valid_tagged_hash(&input.request_private_snapshot_hash) {
        return Err(conflict("HISTORICAL_ARTIFACT_REQUEST_SNAPSHOT_INVALID"));
    }
    let row = locked_receipt_on(
        tx,
        input.receipt_id,
        input.operation_id,
        input.project_scope_id,
        input.principal_id,
    )
    .await?;
    if input.observed_sha256 != row.sha256 || input.observed_byte_len != row.byte_len {
        return Err(conflict("HISTORICAL_ARTIFACT_CONTENT_DRIFT"));
    }
    let status = authority_time_status(&row);
    let attestation_id = Uuid::new_v5(
        &input.receipt_id,
        input.request_private_snapshot_hash.as_bytes(),
    );
    let attestation_hash = tagged_hash(&json!({
        "schema":"historical_report_artifact_read_attestation.v0",
        "attestation_id":attestation_id,
        "receipt_id":input.receipt_id,
        "operation_id":input.operation_id,
        "principal_id":input.principal_id,
        "request_private_snapshot_hash":input.request_private_snapshot_hash,
        "observed_sha256":input.observed_sha256,
        "observed_byte_len":input.observed_byte_len,
        "metadata_manifest_hash":row.metadata_manifest_hash,
        "authority_time_status":authority_time_status_wire(status),
    }));
    let inserted = sqlx::query(
        r#"INSERT INTO historical_report_artifact_read_attestations(
               attestation_id,receipt_id,operation_id,principal_id,
               request_private_snapshot_hash,observed_sha256,observed_byte_len,
               authority_time_status,attestation_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
           ON CONFLICT(receipt_id,request_private_snapshot_hash) DO NOTHING"#,
    )
    .bind(attestation_id)
    .bind(input.receipt_id)
    .bind(input.operation_id)
    .bind(input.principal_id)
    .bind(&input.request_private_snapshot_hash)
    .bind(&input.observed_sha256)
    .bind(input.observed_byte_len)
    .bind(authority_time_status_wire(status))
    .bind(&attestation_hash)
    .execute(&mut **tx)
    .await?;
    if inserted.rows_affected() == 0 {
        let exact: bool = sqlx::query_scalar(
            r#"SELECT attestation_id=$3 AND operation_id=$4 AND principal_id=$5
                      AND observed_sha256=$6 AND observed_byte_len=$7
                      AND authority_time_status=$8 AND attestation_hash=$9
                 FROM historical_report_artifact_read_attestations
                WHERE receipt_id=$1 AND request_private_snapshot_hash=$2 FOR SHARE"#,
        )
        .bind(input.receipt_id)
        .bind(&input.request_private_snapshot_hash)
        .bind(attestation_id)
        .bind(input.operation_id)
        .bind(input.principal_id)
        .bind(&input.observed_sha256)
        .bind(input.observed_byte_len)
        .bind(authority_time_status_wire(status))
        .bind(&attestation_hash)
        .fetch_one(&mut **tx)
        .await?;
        if !exact {
            return Err(conflict("HISTORICAL_ARTIFACT_ATTESTATION_REPLAY_DRIFT"));
        }
    }
    Ok(HistoricalArtifactReadAuthorityV0 {
        historical_artifact_receipt_id: input.receipt_id,
        metadata_manifest_hash: decode_tagged_hash(&row.metadata_manifest_hash)?,
        current_read_attestation_id: attestation_id,
        current_read_attestation_hash: decode_tagged_hash(&attestation_hash)?,
        request_private_snapshot_hash: decode_tagged_hash(&input.request_private_snapshot_hash)?,
        authority_time_status: status,
    })
}

#[derive(Clone, Debug, sqlx::FromRow, Eq, PartialEq)]
pub struct HistoricalArtifactReceiptSourceRow {
    pub receipt_id: Uuid,
    pub operation_id: Uuid,
    pub metadata_manifest_hash: String,
    pub created_at: DateTime<Utc>,
}
