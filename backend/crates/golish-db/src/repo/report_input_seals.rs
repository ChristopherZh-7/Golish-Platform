//! Open -> ordered members -> immutable seal persistence for canonical report inputs.

use golish_memory_domain::source_ref::StoredCanonicalRowId;
use golish_reporting_domain::{ReportInputSealV1, ReportSourceSnapshot};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::Result;

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct ReportInputSealRow {
    pub seal_id: Uuid,
    pub open_id: Uuid,
    pub revision_id: Uuid,
    pub source_member_count: i64,
    pub source_set_hash: Vec<u8>,
    pub report_input_hash: Vec<u8>,
    pub effective_valid_until: chrono::DateTime<chrono::Utc>,
}

fn member_hash(
    ordinal: usize,
    authority_class: &str,
    source_kind: &str,
    source_id_kind: &str,
    source_id_value: &str,
    row_version: i64,
    source_hash: &[u8; 32],
) -> Result<[u8; 32]> {
    Ok(Sha256::digest(serde_json::to_vec(&json!({
        "domain": "report_input_seal_member.v1",
        "ordinal": ordinal,
        "authority_class": authority_class,
        "source_kind": source_kind,
        "source_id_kind": source_id_kind,
        "source_id_value": source_id_value,
        "source_row_version": row_version,
        "source_hash": source_hash,
    }))?)
    .into())
}

pub async fn seal_report_input(
    pool: &PgPool,
    operation_id: Uuid,
    revision_id: Uuid,
    snapshot: &ReportSourceSnapshot,
    seal: &ReportInputSealV1,
) -> Result<ReportInputSealRow> {
    let mut tx = pool.begin().await?;
    let row = seal_report_input_on(&mut tx, operation_id, revision_id, snapshot, seal).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn seal_report_input_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    revision_id: Uuid,
    snapshot: &ReportSourceSnapshot,
    seal: &ReportInputSealV1,
) -> Result<ReportInputSealRow> {
    let (stored_operation_id, stored_source_set_hash, observed_at): (
        Uuid,
        String,
        chrono::DateTime<chrono::Utc>,
    ) = sqlx::query_as(
        r#"SELECT report.operation_id,revision.source_set_hash,transaction_timestamp()
             FROM report_revisions revision
             JOIN reports report USING(report_id)
            WHERE revision.revision_id=$1 FOR SHARE"#,
    )
    .bind(revision_id)
    .fetch_one(&mut **tx)
    .await?;
    if stored_operation_id != operation_id
        || stored_source_set_hash != hex(&snapshot.source_set_hash)
    {
        return Err(anyhow::anyhow!("report_input_source_snapshot_stale").into());
    }
    seal.validate(
        snapshot.ordered_sources.len(),
        snapshot.source_set_hash,
        observed_at,
    )
    .map_err(|code| anyhow::anyhow!(code))?;
    let (tool_truth_authority_set_id, revision_authority_set_id, legacy_authority_seal_id) =
        match seal {
            ReportInputSealV1::RevisionAdjudication(value) => (
                value.report_tool_truth_authority_set.authority_set_id,
                Some(value.revision_adjudication_authority_set.authority_set_id),
                None,
            ),
            ReportInputSealV1::Legacy(value) => (
                value.report_tool_truth_authority_set.authority_set_id,
                None,
                Some(value.legacy_report_authority_seal_id),
            ),
        };
    let effective_valid_until = match seal {
        ReportInputSealV1::RevisionAdjudication(value) => value
            .report_tool_truth_authority_set
            .earliest_effective_valid_until
            .min(
                value
                    .revision_adjudication_authority_set
                    .earliest_effective_valid_until,
            ),
        ReportInputSealV1::Legacy(value) => {
            value
                .report_tool_truth_authority_set
                .earliest_effective_valid_until
        }
    };
    let open_id = Uuid::new_v5(&revision_id, b"report-input-open.v1");
    let seal_id = Uuid::new_v5(&revision_id, b"report-input-seal.v1");
    let report_input_hash = seal.report_input_hash();
    if let Some(existing) = sqlx::query_as::<_, ReportInputSealRow>(
        r#"SELECT seal_id,open_id,revision_id,source_member_count,source_set_hash,
                  report_input_hash,effective_valid_until
             FROM report_input_seals WHERE revision_id=$1"#,
    )
    .bind(revision_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        if existing.seal_id == seal_id
            && existing.open_id == open_id
            && existing.source_member_count
                == i64::try_from(snapshot.ordered_sources.len())
                    .map_err(|_| anyhow::anyhow!("report_input_member_count_overflow"))?
            && existing.source_set_hash == snapshot.source_set_hash
            && existing.report_input_hash == report_input_hash
            && existing.effective_valid_until == effective_valid_until
        {
            return Ok(existing);
        }
        return Err(anyhow::anyhow!("report_input_seal_replay_drift").into());
    }
    let source_member_count = i64::try_from(snapshot.ordered_sources.len())
        .map_err(|_| anyhow::anyhow!("report_input_member_count_overflow"))?;
    let authority_contract = match seal {
        ReportInputSealV1::RevisionAdjudication(_) => "revision_adjudication",
        ReportInputSealV1::Legacy(_) => "legacy",
    };
    sqlx::query(
        r#"INSERT INTO report_input_open_headers(
               open_id,revision_id,operation_id,authority_contract,
               expected_source_member_count,expected_source_set_hash
           ) VALUES($1,$2,$3,$4,$5,$6)"#,
    )
    .bind(open_id)
    .bind(revision_id)
    .bind(operation_id)
    .bind(authority_contract)
    .bind(source_member_count)
    .bind(snapshot.source_set_hash.as_slice())
    .execute(&mut **tx)
    .await?;
    for (ordinal, source) in snapshot.ordered_sources.iter().enumerate() {
        let stored_id = StoredCanonicalRowId::from_domain(&source.id)
            .map_err(|error| anyhow::anyhow!(error.code()))?;
        let hash = member_hash(
            ordinal,
            source.authority_class.as_str(),
            source.kind.as_str(),
            &stored_id.kind,
            &stored_id.value,
            source.row_version,
            &source.content_hash,
        )?;
        sqlx::query(
            r#"INSERT INTO report_input_seal_members(
                   open_id,revision_id,ordinal,authority_class,source_hash,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6)"#,
        )
        .bind(open_id)
        .bind(revision_id)
        .bind(i32::try_from(ordinal).map_err(|_| anyhow::anyhow!("report_input_ordinal_overflow"))?)
        .bind(source.authority_class.as_str())
        .bind(source.content_hash.as_slice())
        .bind(hash.as_slice())
        .execute(&mut **tx)
        .await?;
    }
    let row = sqlx::query_as::<_, ReportInputSealRow>(
        r#"INSERT INTO report_input_seals(
               seal_id,open_id,revision_id,operation_id,tool_truth_authority_set_id,
               revision_adjudication_authority_set_id,legacy_report_authority_seal_id,
               source_member_count,source_set_hash,typed_seal,report_input_hash,
               effective_valid_until
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
           RETURNING seal_id,open_id,revision_id,source_member_count,source_set_hash,
                     report_input_hash,effective_valid_until"#,
    )
    .bind(seal_id)
    .bind(open_id)
    .bind(revision_id)
    .bind(operation_id)
    .bind(tool_truth_authority_set_id)
    .bind(revision_authority_set_id)
    .bind(legacy_authority_seal_id)
    .bind(source_member_count)
    .bind(snapshot.source_set_hash.as_slice())
    .bind(serde_json::to_value(seal)?)
    .bind(report_input_hash.as_slice())
    .bind(effective_valid_until)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
