//! Exact per-organization read-session persistence for unified Investigation.
//!
//! Raw ContextPack bodies never enter these tables.  The repository binds one
//! real stage execution/request to immutable per-organization snapshot,
//! context-chain and transcript partitions, then seals the complete Unit set.

use golish_core::investigation_main_read_session::{
    MainOrganizationReadReceiptV1, MainOrganizationReadSessionV1,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum InvestigationMainSessionStoreError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid unified Investigation main-session input: {0}")]
    InvalidInput(&'static str),
    #[error("unified Investigation main-session identity conflict: {0}")]
    IdentityConflict(&'static str),
    #[error("unified Investigation main-session CAS conflict: {0}")]
    CasConflict(&'static str),
}

pub type InvestigationMainSessionStoreResult<T> = Result<T, InvestigationMainSessionStoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterInvestigationStageAuthority {
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationStageAuthorityRow {
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealInvestigationAnalysisSnapshot {
    pub snapshot_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_sha256: String,
    pub context_item_count: u32,
    pub context_item_set_sha256: String,
    pub methodology_hit_count: u32,
    pub methodology_result_set_sha256: String,
    pub omission_count: u32,
    pub omission_set_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationAnalysisSnapshotRow {
    pub snapshot_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_sha256: String,
    pub context_item_count: i64,
    pub context_item_set_sha256: String,
    pub methodology_hit_count: i64,
    pub methodology_result_set_sha256: String,
    pub omission_count: i64,
    pub omission_set_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginMainSessionSet {
    pub session_set_id: Uuid,
    pub stable_request_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
    pub session_set_ordinal: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct MainSessionSetRow {
    pub session_set_id: Uuid,
    pub stable_request_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
    pub session_set_ordinal: i64,
    pub status: String,
    pub member_count: Option<i64>,
    pub member_set_sha256: Option<String>,
    pub row_version: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct PersistedMainReadSessionRow {
    pub main_read_session_id: Uuid,
    pub session_set_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub snapshot_sha256: String,
    pub context_chain_id: Uuid,
    pub transcript_partition_id: Uuid,
    pub session_contract_version: String,
    pub member_sha256: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct PersistedMainReadReceiptRow {
    pub receipt_id: Uuid,
    pub main_read_session_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub snapshot_sha256: String,
    pub context_item_count: i64,
    pub context_item_set_sha256: String,
    pub methodology_hit_count: i64,
    pub methodology_result_set_sha256: String,
    pub omission_count: i64,
    pub omission_set_sha256: String,
    pub receipt_sha256: String,
}

#[derive(Debug, Serialize)]
struct SessionMemberHashMaterial<'a> {
    main_read_session_id: Uuid,
    session_set_id: Uuid,
    authority_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    owning_stage_run_request_id: &'a str,
    stage_run_unit_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    snapshot_id: Uuid,
    snapshot_sha256: &'a str,
    context_chain_id: Uuid,
    transcript_partition_id: Uuid,
    session_contract_version: &'a str,
}

pub async fn register_stage_authority(
    pool: &PgPool,
    input: &RegisterInvestigationStageAuthority,
) -> InvestigationMainSessionStoreResult<InvestigationStageAuthorityRow> {
    validate_ids(&[
        input.authority_id,
        input.operation_id,
        input.stage_execution_id,
        input.scope_snapshot_id,
    ])?;
    validate_request_id(&input.owning_stage_run_request_id)?;
    sqlx::query(
        r#"INSERT INTO investigation_stage_run_authorities(
               authority_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,scope_snapshot_id
           ) VALUES($1,$2,$3,$4,$5)
           ON CONFLICT(stage_execution_id) DO NOTHING"#,
    )
    .bind(input.authority_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(&input.owning_stage_run_request_id)
    .bind(input.scope_snapshot_id)
    .execute(pool)
    .await?;
    let row = sqlx::query_as::<_, InvestigationStageAuthorityRow>(
        r#"SELECT authority_id,operation_id,stage_execution_id,
                  owning_stage_run_request_id,scope_snapshot_id
             FROM investigation_stage_run_authorities
            WHERE stage_execution_id=$1"#,
    )
    .bind(input.stage_execution_id)
    .fetch_one(pool)
    .await?;
    let expected = InvestigationStageAuthorityRow {
        authority_id: input.authority_id,
        operation_id: input.operation_id,
        stage_execution_id: input.stage_execution_id,
        owning_stage_run_request_id: input.owning_stage_run_request_id.clone(),
        scope_snapshot_id: input.scope_snapshot_id,
    };
    if row != expected {
        return Err(InvestigationMainSessionStoreError::IdentityConflict(
            "stage_authority_replay_mismatch",
        ));
    }
    Ok(row)
}

pub async fn seal_analysis_snapshot(
    pool: &PgPool,
    input: &SealInvestigationAnalysisSnapshot,
) -> InvestigationMainSessionStoreResult<InvestigationAnalysisSnapshotRow> {
    validate_ids(&[
        input.snapshot_id,
        input.authority_id,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
        input.scope_snapshot_id,
        input.organization_id,
    ])?;
    validate_request_id(&input.owning_stage_run_request_id)?;
    for hash in [
        &input.snapshot_sha256,
        &input.context_item_set_sha256,
        &input.methodology_result_set_sha256,
        &input.omission_set_sha256,
    ] {
        validate_sha256(hash)?;
    }
    sqlx::query(
        r#"INSERT INTO investigation_analysis_snapshot_authorities(
               snapshot_id,authority_id,operation_id,stage_execution_id,
               owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,snapshot_sha256,context_item_count,
               context_item_set_sha256,methodology_hit_count,
               methodology_result_set_sha256,omission_count,omission_set_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
           ON CONFLICT(snapshot_id) DO NOTHING"#,
    )
    .bind(input.snapshot_id)
    .bind(input.authority_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(&input.owning_stage_run_request_id)
    .bind(input.stage_run_unit_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id)
    .bind(&input.snapshot_sha256)
    .bind(i64::from(input.context_item_count))
    .bind(&input.context_item_set_sha256)
    .bind(i64::from(input.methodology_hit_count))
    .bind(&input.methodology_result_set_sha256)
    .bind(i64::from(input.omission_count))
    .bind(&input.omission_set_sha256)
    .execute(pool)
    .await?;
    let row = load_snapshot(pool, input.snapshot_id).await?;
    if row.authority_id != input.authority_id
        || row.operation_id != input.operation_id
        || row.stage_execution_id != input.stage_execution_id
        || row.owning_stage_run_request_id != input.owning_stage_run_request_id
        || row.stage_run_unit_id != input.stage_run_unit_id
        || row.scope_snapshot_id != input.scope_snapshot_id
        || row.organization_id != input.organization_id
        || row.snapshot_sha256 != input.snapshot_sha256
        || row.context_item_count != i64::from(input.context_item_count)
        || row.context_item_set_sha256 != input.context_item_set_sha256
        || row.methodology_hit_count != i64::from(input.methodology_hit_count)
        || row.methodology_result_set_sha256 != input.methodology_result_set_sha256
        || row.omission_count != i64::from(input.omission_count)
        || row.omission_set_sha256 != input.omission_set_sha256
    {
        return Err(InvestigationMainSessionStoreError::IdentityConflict(
            "analysis_snapshot_replay_mismatch",
        ));
    }
    Ok(row)
}

pub async fn begin_session_set(
    pool: &PgPool,
    input: &BeginMainSessionSet,
) -> InvestigationMainSessionStoreResult<MainSessionSetRow> {
    validate_ids(&[
        input.session_set_id,
        input.stable_request_id,
        input.authority_id,
        input.operation_id,
        input.stage_execution_id,
        input.scope_snapshot_id,
    ])?;
    validate_request_id(&input.owning_stage_run_request_id)?;
    if input.session_set_ordinal < 0 {
        return Err(InvestigationMainSessionStoreError::InvalidInput(
            "session_set_ordinal",
        ));
    }
    sqlx::query(
        r#"INSERT INTO investigation_main_session_sets(
               session_set_id,stable_request_id,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,
               session_set_ordinal
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)
           ON CONFLICT(stable_request_id) DO NOTHING"#,
    )
    .bind(input.session_set_id)
    .bind(input.stable_request_id)
    .bind(input.authority_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(&input.owning_stage_run_request_id)
    .bind(input.scope_snapshot_id)
    .bind(input.session_set_ordinal)
    .execute(pool)
    .await?;
    let row = load_set_by_request(pool, input.stable_request_id).await?;
    if row.session_set_id != input.session_set_id
        || row.authority_id != input.authority_id
        || row.operation_id != input.operation_id
        || row.stage_execution_id != input.stage_execution_id
        || row.owning_stage_run_request_id != input.owning_stage_run_request_id
        || row.scope_snapshot_id != input.scope_snapshot_id
        || row.session_set_ordinal != input.session_set_ordinal
    {
        return Err(InvestigationMainSessionStoreError::IdentityConflict(
            "session_set_replay_mismatch",
        ));
    }
    Ok(row)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_read_session(
    pool: &PgPool,
    session_set_id: Uuid,
    authority_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    scope_snapshot_id: Uuid,
    session: &MainOrganizationReadSessionV1,
) -> InvestigationMainSessionStoreResult<PersistedMainReadSessionRow> {
    validate_ids(&[
        session_set_id,
        authority_id,
        operation_id,
        stage_execution_id,
        scope_snapshot_id,
        session.main_read_session_id,
    ])?;
    if session.operation_id != operation_id {
        return Err(InvestigationMainSessionStoreError::IdentityConflict(
            "read_session_operation_mismatch",
        ));
    }
    if session.stage_execution_id != stage_execution_id {
        return Err(InvestigationMainSessionStoreError::IdentityConflict(
            "read_session_stage_execution_mismatch",
        ));
    }
    let member_sha256 = sha256_json(&SessionMemberHashMaterial {
        main_read_session_id: session.main_read_session_id,
        session_set_id,
        authority_id,
        operation_id,
        stage_execution_id,
        owning_stage_run_request_id: &session.owning_stage_run_request_id,
        stage_run_unit_id: session.stage_run_unit_id,
        scope_snapshot_id,
        organization_id: session.organization_id,
        snapshot_id: session.snapshot_id,
        snapshot_sha256: &session.snapshot_sha256,
        context_chain_id: session.context_chain_id,
        transcript_partition_id: session.transcript_partition_id,
        session_contract_version: &session.session_contract_version,
    });
    sqlx::query(
        r#"INSERT INTO investigation_main_read_sessions(
               main_read_session_id,session_set_id,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
               scope_snapshot_id,organization_id,snapshot_id,snapshot_sha256,
               context_chain_id,transcript_partition_id,session_contract_version,
               member_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
           ON CONFLICT(main_read_session_id) DO NOTHING"#,
    )
    .bind(session.main_read_session_id)
    .bind(session_set_id)
    .bind(authority_id)
    .bind(operation_id)
    .bind(stage_execution_id)
    .bind(&session.owning_stage_run_request_id)
    .bind(session.stage_run_unit_id)
    .bind(scope_snapshot_id)
    .bind(session.organization_id)
    .bind(session.snapshot_id)
    .bind(&session.snapshot_sha256)
    .bind(session.context_chain_id)
    .bind(session.transcript_partition_id)
    .bind(&session.session_contract_version)
    .bind(&member_sha256)
    .execute(pool)
    .await?;
    let row = load_read_session(pool, session.main_read_session_id).await?;
    if row.session_set_id != session_set_id
        || row.authority_id != authority_id
        || row.operation_id != operation_id
        || row.stage_execution_id != stage_execution_id
        || row.owning_stage_run_request_id != session.owning_stage_run_request_id
        || row.stage_run_unit_id != session.stage_run_unit_id
        || row.scope_snapshot_id != scope_snapshot_id
        || row.organization_id != session.organization_id
        || row.snapshot_id != session.snapshot_id
        || row.snapshot_sha256 != session.snapshot_sha256
        || row.context_chain_id != session.context_chain_id
        || row.transcript_partition_id != session.transcript_partition_id
        || row.session_contract_version != session.session_contract_version
        || row.member_sha256 != member_sha256
    {
        return Err(InvestigationMainSessionStoreError::IdentityConflict(
            "read_session_replay_mismatch",
        ));
    }
    Ok(row)
}

pub async fn record_read_receipt(
    pool: &PgPool,
    receipt_id: Uuid,
    receipt: &MainOrganizationReadReceiptV1,
) -> InvestigationMainSessionStoreResult<()> {
    validate_ids(&[receipt_id, receipt.main_read_session_id])?;
    let session_row = load_read_session(pool, receipt.main_read_session_id).await?;
    let session = MainOrganizationReadSessionV1 {
        main_read_session_id: session_row.main_read_session_id,
        operation_id: session_row.operation_id,
        stage_execution_id: session_row.stage_execution_id,
        owning_stage_run_request_id: session_row.owning_stage_run_request_id.clone(),
        stage_run_unit_id: session_row.stage_run_unit_id,
        organization_id: session_row.organization_id,
        snapshot_id: session_row.snapshot_id,
        snapshot_sha256: session_row.snapshot_sha256.clone(),
        context_chain_id: session_row.context_chain_id,
        transcript_partition_id: session_row.transcript_partition_id,
        session_contract_version: session_row.session_contract_version.clone(),
    };
    let expected = session
        .host_receipt(
            receipt.context_item_count,
            receipt.context_item_set_sha256.clone(),
            receipt.methodology_hit_count,
            receipt.methodology_result_set_sha256.clone(),
            receipt.omission_count,
            receipt.omission_set_sha256.clone(),
        )
        .map_err(|_| InvestigationMainSessionStoreError::InvalidInput("receipt"))?;
    if &expected != receipt {
        return Err(InvestigationMainSessionStoreError::IdentityConflict(
            "read_receipt_hash_mismatch",
        ));
    }
    sqlx::query(
        r#"INSERT INTO investigation_main_read_session_receipts(
               receipt_id,main_read_session_id,operation_id,stage_execution_id,
               stage_run_unit_id,organization_id,snapshot_id,snapshot_sha256,
               context_item_count,context_item_set_sha256,methodology_hit_count,
               methodology_result_set_sha256,omission_count,omission_set_sha256,
               receipt_sha256
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
           ON CONFLICT(main_read_session_id) DO NOTHING"#,
    )
    .bind(receipt_id)
    .bind(receipt.main_read_session_id)
    .bind(session_row.operation_id)
    .bind(session_row.stage_execution_id)
    .bind(session_row.stage_run_unit_id)
    .bind(session_row.organization_id)
    .bind(receipt.snapshot_id)
    .bind(&receipt.snapshot_sha256)
    .bind(i64::from(receipt.context_item_count))
    .bind(&receipt.context_item_set_sha256)
    .bind(i64::from(receipt.methodology_hit_count))
    .bind(&receipt.methodology_result_set_sha256)
    .bind(i64::from(receipt.omission_count))
    .bind(&receipt.omission_set_sha256)
    .bind(&receipt.receipt_sha256)
    .execute(pool)
    .await?;
    let stored: (Uuid, String) = sqlx::query_as(
        "SELECT receipt_id,receipt_sha256 FROM investigation_main_read_session_receipts WHERE main_read_session_id=$1",
    )
    .bind(receipt.main_read_session_id)
    .fetch_one(pool)
    .await?;
    if stored != (receipt_id, receipt.receipt_sha256.clone()) {
        return Err(InvestigationMainSessionStoreError::IdentityConflict(
            "read_receipt_replay_mismatch",
        ));
    }
    Ok(())
}

pub async fn seal_session_set(
    pool: &PgPool,
    session_set_id: Uuid,
    expected_row_version: i64,
) -> InvestigationMainSessionStoreResult<MainSessionSetRow> {
    let mut tx = pool.begin().await?;
    let current = load_set_for_update(&mut tx, session_set_id).await?;
    if current.status == "sealed" {
        tx.commit().await?;
        return Ok(current);
    }
    if current.status != "open" || current.row_version != expected_row_version {
        return Err(InvestigationMainSessionStoreError::CasConflict(
            "session_set_head",
        ));
    }
    let (count, set_hash): (i64, String) = sqlx::query_as(
        r#"SELECT COUNT(*),unified_investigation_exact_set_hash(
               'investigation_main_read_sessions.v1',
               COALESCE(array_agg(
                   main_read_session_id::TEXT || ':' || member_sha256
                   ORDER BY organization_id,main_read_session_id
               ),ARRAY[]::TEXT[])
           ) FROM investigation_main_read_sessions WHERE session_set_id=$1"#,
    )
    .bind(session_set_id)
    .fetch_one(&mut *tx)
    .await?;
    let row = sqlx::query_as::<_, MainSessionSetRow>(
        r#"UPDATE investigation_main_session_sets
              SET status='sealed',member_count=$3,member_set_sha256=$4,
                  row_version=row_version+1,sealed_at=statement_timestamp()
            WHERE session_set_id=$1 AND status='open' AND row_version=$2
            RETURNING session_set_id,stable_request_id,authority_id,operation_id,
                      stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,
                      session_set_ordinal,status,member_count,member_set_sha256,row_version"#,
    )
    .bind(session_set_id)
    .bind(expected_row_version)
    .bind(count)
    .bind(set_hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(InvestigationMainSessionStoreError::CasConflict(
        "session_set_head",
    ))?;
    tx.commit().await?;
    Ok(row)
}

async fn load_snapshot(
    pool: &PgPool,
    snapshot_id: Uuid,
) -> InvestigationMainSessionStoreResult<InvestigationAnalysisSnapshotRow> {
    Ok(sqlx::query_as::<_, InvestigationAnalysisSnapshotRow>(
        r#"SELECT snapshot_id,authority_id,operation_id,stage_execution_id,
                  owning_stage_run_request_id,stage_run_unit_id,scope_snapshot_id,
                  organization_id,snapshot_sha256,context_item_count,
                  context_item_set_sha256,methodology_hit_count,
                  methodology_result_set_sha256,omission_count,omission_set_sha256
             FROM investigation_analysis_snapshot_authorities WHERE snapshot_id=$1"#,
    )
    .bind(snapshot_id)
    .fetch_one(pool)
    .await?)
}

async fn load_set_by_request(
    pool: &PgPool,
    stable_request_id: Uuid,
) -> InvestigationMainSessionStoreResult<MainSessionSetRow> {
    Ok(sqlx::query_as::<_, MainSessionSetRow>(
        r#"SELECT session_set_id,stable_request_id,authority_id,operation_id,
                  stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,
                  session_set_ordinal,status,member_count,member_set_sha256,row_version
             FROM investigation_main_session_sets WHERE stable_request_id=$1"#,
    )
    .bind(stable_request_id)
    .fetch_one(pool)
    .await?)
}

async fn load_set_for_update(
    tx: &mut Transaction<'_, Postgres>,
    session_set_id: Uuid,
) -> InvestigationMainSessionStoreResult<MainSessionSetRow> {
    Ok(sqlx::query_as::<_, MainSessionSetRow>(
        r#"SELECT session_set_id,stable_request_id,authority_id,operation_id,
                  stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,
                  session_set_ordinal,status,member_count,member_set_sha256,row_version
             FROM investigation_main_session_sets WHERE session_set_id=$1 FOR UPDATE"#,
    )
    .bind(session_set_id)
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn load_read_session(
    pool: &PgPool,
    main_read_session_id: Uuid,
) -> InvestigationMainSessionStoreResult<PersistedMainReadSessionRow> {
    Ok(sqlx::query_as::<_, PersistedMainReadSessionRow>(
        r#"SELECT main_read_session_id,session_set_id,authority_id,operation_id,
                  stage_execution_id,owning_stage_run_request_id,stage_run_unit_id,
                  scope_snapshot_id,organization_id,snapshot_id,snapshot_sha256,
                  context_chain_id,transcript_partition_id,session_contract_version,
                  member_sha256
             FROM investigation_main_read_sessions WHERE main_read_session_id=$1"#,
    )
    .bind(main_read_session_id)
    .fetch_one(pool)
    .await?)
}

pub async fn load_read_receipt(
    pool: &PgPool,
    main_read_session_id: Uuid,
) -> InvestigationMainSessionStoreResult<PersistedMainReadReceiptRow> {
    Ok(sqlx::query_as::<_, PersistedMainReadReceiptRow>(
        r#"SELECT receipt_id,main_read_session_id,operation_id,stage_execution_id,
                  stage_run_unit_id,organization_id,snapshot_id,snapshot_sha256,
                  context_item_count,context_item_set_sha256,methodology_hit_count,
                  methodology_result_set_sha256,omission_count,omission_set_sha256,
                  receipt_sha256
             FROM investigation_main_read_session_receipts
            WHERE main_read_session_id=$1"#,
    )
    .bind(main_read_session_id)
    .fetch_one(pool)
    .await?)
}

fn validate_ids(values: &[Uuid]) -> InvestigationMainSessionStoreResult<()> {
    if values.iter().any(Uuid::is_nil) {
        return Err(InvestigationMainSessionStoreError::InvalidInput("uuid"));
    }
    Ok(())
}

fn validate_request_id(value: &str) -> InvestigationMainSessionStoreResult<()> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(InvestigationMainSessionStoreError::InvalidInput(
            "owning_stage_run_request_id",
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> InvestigationMainSessionStoreResult<()> {
    let valid = value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    });
    if !valid {
        return Err(InvestigationMainSessionStoreError::InvalidInput("sha256"));
    }
    Ok(())
}

fn sha256_json(value: &impl Serialize) -> String {
    let digest = Sha256::digest(
        serde_json::to_vec(value).expect("main-session hash material is serializable"),
    );
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
