use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct UpsertKnowledgeDocument {
    pub document_id: Uuid,
    pub document_key: String,
    pub project_scope_id: Option<Uuid>,
    pub source_stream_key: String,
    pub source_version: i64,
    pub projection_schema_version: i32,
    pub redaction_policy_version: i32,
    pub assertion_ids: Vec<Uuid>,
    pub document_type: String,
    pub redacted_content: String,
    pub content_hash: String,
    pub classification: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct KnowledgeDocumentRow {
    pub document_id: Uuid,
    pub document_key: String,
    pub project_scope_id: Option<Uuid>,
    pub source_stream_key: String,
    pub source_version: i64,
    pub projection_schema_version: i32,
    pub redaction_policy_version: i32,
    pub assertion_ids: Vec<Uuid>,
    pub status: String,
    pub document_type: String,
    pub redacted_content: String,
    pub content_hash: String,
    pub classification: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
}

pub async fn upsert(
    pool: &PgPool,
    document: &UpsertKnowledgeDocument,
) -> Result<KnowledgeDocumentRow, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let row = upsert_with_connection(&mut tx, document).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn upsert_with_connection(
    connection: &mut PgConnection,
    document: &UpsertKnowledgeDocument,
) -> Result<KnowledgeDocumentRow, sqlx::Error> {
    sqlx::query_as(
        r#"INSERT INTO knowledge_documents (
               document_id, document_key, project_scope_id, source_stream_key,
               source_version, projection_schema_version, redaction_policy_version,
               assertion_ids, status, document_type, redacted_content, content_hash,
               classification, valid_from, valid_to
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'active',$9,$10,$11,$12,$13,$14)
           ON CONFLICT (
               project_scope_id, source_stream_key, source_version,
               redaction_policy_version
           ) DO UPDATE SET
               document_key = EXCLUDED.document_key,
               projection_schema_version = EXCLUDED.projection_schema_version,
               assertion_ids = EXCLUDED.assertion_ids,
               status = 'active',
               document_type = EXCLUDED.document_type,
               redacted_content = EXCLUDED.redacted_content,
               content_hash = EXCLUDED.content_hash,
               classification = EXCLUDED.classification,
               valid_from = EXCLUDED.valid_from,
               valid_to = EXCLUDED.valid_to,
               updated_at = NOW()
           RETURNING document_id, document_key, project_scope_id,
                     source_stream_key, source_version, projection_schema_version,
                     redaction_policy_version, assertion_ids, status, document_type,
                     redacted_content, content_hash, classification, valid_from,
                     valid_to"#,
    )
    .bind(document.document_id)
    .bind(&document.document_key)
    .bind(document.project_scope_id)
    .bind(&document.source_stream_key)
    .bind(document.source_version)
    .bind(document.projection_schema_version)
    .bind(document.redaction_policy_version)
    .bind(&document.assertion_ids)
    .bind(&document.document_type)
    .bind(&document.redacted_content)
    .bind(&document.content_hash)
    .bind(&document.classification)
    .bind(document.valid_from)
    .bind(document.valid_to)
    .fetch_one(&mut *connection)
    .await
}

/// Closes documents and every derived embedding in one caller-owned
/// transaction. Historical rows are retained.
pub async fn invalidate_source(
    pool: &PgPool,
    project_scope_id: Option<Uuid>,
    source_stream_key: &str,
    source_version: i64,
    invalidated_at: DateTime<Utc>,
) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let count = invalidate_source_with_connection(
        &mut tx,
        project_scope_id,
        source_stream_key,
        source_version,
        invalidated_at,
    )
    .await?;
    tx.commit().await?;
    Ok(count)
}

pub async fn invalidate_source_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    project_scope_id: Option<Uuid>,
    source_stream_key: &str,
    source_version: i64,
    invalidated_at: DateTime<Utc>,
) -> Result<u64, sqlx::Error> {
    invalidate_source_with_connection(
        tx,
        project_scope_id,
        source_stream_key,
        source_version,
        invalidated_at,
    )
    .await
}

pub async fn invalidate_source_with_connection(
    connection: &mut PgConnection,
    project_scope_id: Option<Uuid>,
    source_stream_key: &str,
    source_version: i64,
    invalidated_at: DateTime<Utc>,
) -> Result<u64, sqlx::Error> {
    let document_ids = sqlx::query_scalar::<_, Uuid>(
        r#"UPDATE knowledge_documents
           SET status = 'invalidated',
               valid_to = LEAST(COALESCE(valid_to, $4), $4),
               updated_at = NOW()
           WHERE project_scope_id IS NOT DISTINCT FROM $1
             AND source_stream_key = $2
             AND source_version = $3
             AND status <> 'invalidated'
           RETURNING document_id"#,
    )
    .bind(project_scope_id)
    .bind(source_stream_key)
    .bind(source_version)
    .bind(invalidated_at)
    .fetch_all(&mut *connection)
    .await?;
    if !document_ids.is_empty() {
        sqlx::query(
            r#"UPDATE knowledge_embeddings
               SET status = 'invalidated',
                   valid_to = LEAST(COALESCE(valid_to, $2), $2),
                   updated_at = NOW()
               WHERE document_id = ANY($1) AND status <> 'invalidated'"#,
        )
        .bind(&document_ids)
        .bind(invalidated_at)
        .execute(&mut *connection)
        .await?;
    }
    Ok(document_ids.len() as u64)
}

pub async fn list_active_for_source(
    pool: &PgPool,
    project_scope_id: Option<Uuid>,
    source_stream_key: &str,
    source_version: i64,
) -> Result<Vec<KnowledgeDocumentRow>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT document_id, document_key, project_scope_id,
                  source_stream_key, source_version, projection_schema_version,
                  redaction_policy_version, assertion_ids, status, document_type,
                  redacted_content, content_hash, classification, valid_from,
                  valid_to
           FROM knowledge_documents
           WHERE project_scope_id IS NOT DISTINCT FROM $1
             AND source_stream_key = $2
             AND source_version = $3
             AND status = 'active'
           ORDER BY redaction_policy_version, document_id"#,
    )
    .bind(project_scope_id)
    .bind(source_stream_key)
    .bind(source_version)
    .fetch_all(pool)
    .await
}

pub async fn get(pool: &PgPool, document_id: Uuid) -> Result<KnowledgeDocumentRow, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT document_id, document_key, project_scope_id,
                  source_stream_key, source_version, projection_schema_version,
                  redaction_policy_version, assertion_ids, status, document_type,
                  redacted_content, content_hash, classification, valid_from,
                  valid_to
           FROM knowledge_documents WHERE document_id = $1"#,
    )
    .bind(document_id)
    .fetch_one(pool)
    .await
}
