use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{AgentType, Memory, MemoryType, NewMemory};

/// Result of a similarity search, including the match score.
#[derive(Debug, Clone)]
pub struct ScoredMemory {
    pub memory: Memory,
    pub score: f32,
}

// ---------------------------------------------------------------------------
// Write operations
// ---------------------------------------------------------------------------

/// Store a memory with optional embedding vector.
pub async fn store(pool: &PgPool, m: NewMemory) -> Result<Memory> {
    let emb_bytes: Option<Vec<u8>> = m.embedding.as_ref().map(|v| {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    });
    let emb_dim: Option<i32> = m.embedding.as_ref().map(|v| v.len() as i32);

    let row = sqlx::query_as::<_, Memory>(
        r#"INSERT INTO memories
               (session_id, task_id, subtask_id, agent, content, mem_type,
                tool_name, doc_type, embedding, embedding_dim, metadata)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           RETURNING id, session_id, task_id, subtask_id, agent, content,
                     mem_type, tool_name, doc_type, embedding_dim, metadata, created_at"#,
    )
    .bind(m.session_id)
    .bind(m.task_id)
    .bind(m.subtask_id)
    .bind(m.agent)
    .bind(&m.content)
    .bind(m.mem_type)
    .bind(&m.tool_name)
    .bind(&m.doc_type)
    .bind(&emb_bytes)
    .bind(emb_dim)
    .bind(&m.metadata)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Vector similarity search (Rust-side cosine similarity)
// ---------------------------------------------------------------------------

/// Internal struct to load a candidate row with its raw BYTEA embedding.
#[derive(Debug, sqlx::FromRow)]
struct CandidateRow {
    id: Uuid,
    session_id: Option<Uuid>,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    agent: Option<AgentType>,
    content: String,
    mem_type: MemoryType,
    tool_name: Option<String>,
    doc_type: String,
    #[allow(dead_code)]
    project_path: Option<String>,
    embedding_dim: Option<i32>,
    metadata: Option<serde_json::Value>,
    created_at: chrono::DateTime<chrono::Utc>,
    embedding: Option<Vec<u8>>,
}

/// Semantic similarity search using cosine similarity computed in Rust.
///
/// Loads candidate vectors from the DB (filtered by session/type/doc_type),
/// computes cosine similarity against the query embedding, and returns
/// top-k results above the threshold.
pub async fn search_similar(
    pool: &PgPool,
    query_embedding: &[f32],
    filters: &MemoryFilters,
    limit: usize,
    threshold: f32,
) -> Result<Vec<ScoredMemory>> {
    let candidates = load_candidates(pool, filters).await?;

    let mut scored: Vec<ScoredMemory> = candidates
        .into_iter()
        .filter_map(|row| {
            let emb = decode_embedding(&row.embedding?, row.embedding_dim?)?;
            let score = cosine_similarity(query_embedding, &emb);
            if score < threshold {
                return None;
            }
            Some(ScoredMemory {
                memory: Memory {
                    id: row.id,
                    session_id: row.session_id,
                    task_id: row.task_id,
                    subtask_id: row.subtask_id,
                    agent: row.agent,
                    content: row.content,
                    mem_type: row.mem_type,
                    tool_name: row.tool_name,
                    doc_type: row.doc_type,
                    project_path: row.project_path,
                    embedding_dim: row.embedding_dim,
                    metadata: row.metadata,
                    created_at: row.created_at,
                },
                score,
            })
        })
        .collect();

    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    Ok(scored)
}

/// Filters for narrowing down memory search candidates.
#[derive(Debug, Default)]
pub struct MemoryFilters {
    pub session_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub mem_type: Option<MemoryType>,
    pub doc_type: Option<String>,
    /// When set, only returns memories for this project (+ global memories where project_path IS NULL).
    pub project_path: Option<String>,
}

async fn load_candidates(pool: &PgPool, f: &MemoryFilters) -> Result<Vec<CandidateRow>> {
    let rows = sqlx::query_as::<_, CandidateRow>(
        r#"SELECT id, session_id, task_id, subtask_id, agent, content,
                  mem_type, tool_name, doc_type, project_path, embedding_dim,
                  metadata, created_at, embedding
           FROM memories
           WHERE embedding IS NOT NULL
             AND ($1::uuid IS NULL OR session_id = $1)
             AND ($2::uuid IS NULL OR task_id = $2)
             AND ($3::uuid IS NULL OR subtask_id = $3)
             AND ($4::memory_type IS NULL OR mem_type = $4)
             AND ($5::text IS NULL OR doc_type = $5)
             AND ($6::text IS NULL OR project_path = $6 OR project_path IS NULL)
           ORDER BY created_at DESC
           LIMIT 500"#,
    )
    .bind(f.session_id)
    .bind(f.task_id)
    .bind(f.subtask_id)
    .bind(f.mem_type)
    .bind(&f.doc_type)
    .bind(&f.project_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

fn decode_embedding(bytes: &[u8], dim: i32) -> Option<Vec<f32>> {
    let dim = dim as usize;
    if bytes.len() != dim * 4 {
        return None;
    }
    let mut vec = Vec::with_capacity(dim);
    for chunk in bytes.chunks_exact(4) {
        vec.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Some(vec)
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut norm_a, mut norm_b) = (0.0_f32, 0.0_f32, 0.0_f32);
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 { 0.0 } else { dot / denom }
}

// ---------------------------------------------------------------------------
// Text-based fallback search
// ---------------------------------------------------------------------------

/// Text-based memory search using ILIKE pattern matching.
pub async fn search_text(
    pool: &PgPool,
    query: &str,
    mem_type: Option<MemoryType>,
    limit: i64,
) -> Result<Vec<Memory>> {
    let pattern = format!("%{}%", query);

    let rows = if let Some(mt) = mem_type {
        sqlx::query_as::<_, Memory>(
            r#"SELECT id, session_id, task_id, subtask_id, agent, content,
                      mem_type, tool_name, doc_type, project_path, embedding_dim, metadata, created_at
               FROM memories
               WHERE content ILIKE $1 AND mem_type = $2
               ORDER BY created_at DESC LIMIT $3"#,
        )
        .bind(&pattern)
        .bind(mt)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Memory>(
            r#"SELECT id, session_id, task_id, subtask_id, agent, content,
                      mem_type, tool_name, doc_type, project_path, embedding_dim, metadata, created_at
               FROM memories
               WHERE content ILIKE $1
               ORDER BY created_at DESC LIMIT $2"#,
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Basic CRUD
// ---------------------------------------------------------------------------

pub async fn list_by_type(pool: &PgPool, mem_type: MemoryType, limit: i64) -> Result<Vec<Memory>> {
    let rows = sqlx::query_as::<_, Memory>(
        r#"SELECT id, session_id, task_id, subtask_id, agent, content,
                  mem_type, tool_name, doc_type, project_path, embedding_dim, metadata, created_at
           FROM memories WHERE mem_type = $1
           ORDER BY created_at DESC LIMIT $2"#,
    )
    .bind(mem_type)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_recent(pool: &PgPool, limit: i64) -> Result<Vec<Memory>> {
    let rows = sqlx::query_as::<_, Memory>(
        r#"SELECT id, session_id, task_id, subtask_id, agent, content,
                  mem_type, tool_name, doc_type, project_path, embedding_dim, metadata, created_at
           FROM memories
           ORDER BY created_at DESC LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM memories WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn count(pool: &PgPool) -> Result<i64> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM memories")
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 2.0, 3.0];
        let b: Vec<f32> = a.iter().map(|x| -x).collect();
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_decode_embedding_roundtrip() {
        let original = vec![1.0_f32, -2.5, 0.333];
        let bytes: Vec<u8> = original.iter().flat_map(|f| f.to_le_bytes()).collect();
        let decoded = decode_embedding(&bytes, 3).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_decode_embedding_wrong_dim() {
        let bytes = vec![0u8; 12]; // 3 floats
        assert!(decode_embedding(&bytes, 4).is_none());
    }
}
