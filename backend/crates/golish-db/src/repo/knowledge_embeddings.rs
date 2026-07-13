use chrono::{DateTime, Utc};
use golish_memory_domain::embedding::validate_embedding_dimension;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct InsertKnowledgeEmbedding {
    pub embedding_id: Uuid,
    pub document_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub provider: String,
    pub model: String,
    pub embedding: Vec<f32>,
    pub content_hash: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct KnowledgeEmbeddingRow {
    pub embedding_id: Uuid,
    pub document_id: Uuid,
    pub source_stream_key: String,
    pub source_version: i64,
    pub status: String,
    pub provider: String,
    pub model: String,
    pub embedding_dimension: i32,
    pub embedding_schema_version: i32,
    pub content_hash: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
}

pub async fn insert(
    pool: &PgPool,
    input: &InsertKnowledgeEmbedding,
) -> Result<KnowledgeEmbeddingRow, KnowledgeEmbeddingError> {
    let mut connection = pool.acquire().await?;
    insert_with_connection(&mut connection, input).await
}

pub async fn insert_with_connection(
    connection: &mut PgConnection,
    input: &InsertKnowledgeEmbedding,
) -> Result<KnowledgeEmbeddingRow, KnowledgeEmbeddingError> {
    validate_embedding_dimension(input.embedding.len())?;
    if input.embedding.iter().any(|value| !value.is_finite()) {
        return Err(KnowledgeEmbeddingError::NonFiniteValue);
    }
    let vector = format!(
        "[{}]",
        input
            .embedding
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    Ok(sqlx::query_as(
        r#"INSERT INTO knowledge_embeddings (
               embedding_id, document_id, source_stream_key, source_version,
               status, provider, model, embedding, embedding_dimension,
               embedding_schema_version, content_hash, valid_from, valid_to
           ) VALUES ($1,$2,$3,$4,'active',$5,$6,$7::vector,1536,1,$8,$9,$10)
           ON CONFLICT (
               document_id, provider, model, embedding_schema_version, content_hash
           ) DO UPDATE SET
               status = 'active',
               embedding = EXCLUDED.embedding,
               valid_from = EXCLUDED.valid_from,
               valid_to = EXCLUDED.valid_to,
               updated_at = NOW()
           RETURNING embedding_id, document_id, source_stream_key,
                     source_version, status, provider, model,
                     embedding_dimension, embedding_schema_version, content_hash,
                     valid_from, valid_to"#,
    )
    .bind(input.embedding_id)
    .bind(input.document_id)
    .bind(&input.source_stream_key)
    .bind(input.source_version)
    .bind(&input.provider)
    .bind(&input.model)
    .bind(vector)
    .bind(&input.content_hash)
    .bind(input.valid_from)
    .bind(input.valid_to)
    .fetch_one(&mut *connection)
    .await?)
}

pub async fn get(pool: &PgPool, embedding_id: Uuid) -> Result<KnowledgeEmbeddingRow, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT embedding_id, document_id, source_stream_key, source_version,
                  status, provider, model, embedding_dimension,
                  embedding_schema_version, content_hash, valid_from, valid_to
           FROM knowledge_embeddings WHERE embedding_id = $1"#,
    )
    .bind(embedding_id)
    .fetch_one(pool)
    .await
}

#[derive(Debug, thiserror::Error)]
pub enum KnowledgeEmbeddingError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Dimension(#[from] golish_memory_domain::embedding::EmbeddingDimensionError),
    #[error("embedding contains a non-finite value")]
    NonFiniteValue,
}

impl KnowledgeEmbeddingError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Sqlx(_) => "memory_embedding_database_error",
            Self::Dimension(error) => error.code(),
            Self::NonFiniteValue => "memory_embedding_non_finite",
        }
    }
}
