use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{AgentType, Memory, MemoryType, NewMemory};

#[derive(Debug, Clone)]
pub struct ScoredMemory {
    pub memory: Memory,
    pub score: f32,
}

// ---------------------------------------------------------------------------
// Write operations
// ---------------------------------------------------------------------------

/// Store a memory. Pass `has_pgvector = true` when the vector extension is loaded
/// to cast embeddings as `vector(1536)`, otherwise stores as BYTEA.
pub async fn store(pool: &PgPool, m: NewMemory, has_pgvector: bool) -> Result<Memory> {
    let returning = r#"RETURNING id, session_id, task_id, subtask_id, agent, content,
                     mem_type, tool_name, doc_type, project_path, metadata, created_at"#;

    if has_pgvector {
        let emb_str: Option<String> = m.embedding.as_ref().map(|v| {
            let parts: Vec<String> = v.iter().map(|f| f.to_string()).collect();
            format!("[{}]", parts.join(","))
        });

        let row = sqlx::query_as::<_, Memory>(&format!(
            r#"INSERT INTO memories
                   (session_id, task_id, subtask_id, agent, content, mem_type,
                    tool_name, doc_type, project_path, embedding, metadata)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::vector, $11)
               {returning}"#
        ))
        .bind(m.session_id)
        .bind(m.task_id)
        .bind(m.subtask_id)
        .bind(m.agent)
        .bind(&m.content)
        .bind(m.mem_type)
        .bind(&m.tool_name)
        .bind(&m.doc_type)
        .bind(&m.project_path)
        .bind(&emb_str)
        .bind(&m.metadata)
        .fetch_one(pool)
        .await?;
        Ok(row)
    } else {
        let emb_bytes: Option<Vec<u8>> = m.embedding.as_ref().map(|v| {
            v.iter().flat_map(|f| f.to_le_bytes()).collect()
        });

        let row = sqlx::query_as::<_, Memory>(&format!(
            r#"INSERT INTO memories
                   (session_id, task_id, subtask_id, agent, content, mem_type,
                    tool_name, doc_type, project_path, embedding, metadata)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
               {returning}"#
        ))
        .bind(m.session_id)
        .bind(m.task_id)
        .bind(m.subtask_id)
        .bind(m.agent)
        .bind(&m.content)
        .bind(m.mem_type)
        .bind(&m.tool_name)
        .bind(&m.doc_type)
        .bind(&m.project_path)
        .bind(&emb_bytes)
        .bind(&m.metadata)
        .fetch_one(pool)
        .await?;
        Ok(row)
    }
}

// ---------------------------------------------------------------------------
// Vector similarity search (pgvector native)
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct MemoryFilters {
    pub session_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub mem_type: Option<MemoryType>,
    pub doc_type: Option<String>,
    pub project_path: Option<String>,
}

/// Semantic similarity search using pgvector's cosine distance operator.
/// Returns top-`limit` results above the similarity `threshold` (0.0–1.0).
pub async fn search_similar(
    pool: &PgPool,
    query_embedding: &[f32],
    filters: &MemoryFilters,
    limit: usize,
    threshold: f32,
) -> Result<Vec<ScoredMemory>> {
    let emb_str = {
        let parts: Vec<String> = query_embedding.iter().map(|f| f.to_string()).collect();
        format!("[{}]", parts.join(","))
    };

    // pgvector `<=>` returns cosine distance (0 = identical, 2 = opposite).
    // We convert to similarity: 1 - distance.
    let rows = sqlx::query_as::<_, ScoredRow>(
        r#"SELECT id, session_id, task_id, subtask_id, agent, content,
                  mem_type, tool_name, doc_type, project_path, metadata, created_at,
                  1.0 - (embedding <=> $1::vector) AS score
           FROM memories
           WHERE embedding IS NOT NULL
             AND ($2::uuid IS NULL OR session_id = $2)
             AND ($3::uuid IS NULL OR task_id = $3)
             AND ($4::uuid IS NULL OR subtask_id = $4)
             AND ($5::memory_type IS NULL OR mem_type = $5)
             AND ($6::text IS NULL OR doc_type = $6)
             AND ($7::text IS NULL OR project_path = $7 OR project_path IS NULL)
           ORDER BY embedding <=> $1::vector ASC
           LIMIT $8"#,
    )
    .bind(&emb_str)
    .bind(filters.session_id)
    .bind(filters.task_id)
    .bind(filters.subtask_id)
    .bind(filters.mem_type)
    .bind(&filters.doc_type)
    .bind(&filters.project_path)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    let results = rows
        .into_iter()
        .filter(|r| r.score >= threshold)
        .map(|r| ScoredMemory {
            memory: Memory {
                id: r.id,
                session_id: r.session_id,
                task_id: r.task_id,
                subtask_id: r.subtask_id,
                agent: r.agent,
                content: r.content,
                mem_type: r.mem_type,
                tool_name: r.tool_name,
                doc_type: r.doc_type,
                project_path: r.project_path,
                metadata: r.metadata,
                created_at: r.created_at,
            },
            score: r.score,
        })
        .collect();

    Ok(results)
}

#[derive(Debug, sqlx::FromRow)]
struct ScoredRow {
    id: Uuid,
    session_id: Option<Uuid>,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    agent: Option<AgentType>,
    content: String,
    mem_type: MemoryType,
    tool_name: Option<String>,
    doc_type: String,
    project_path: Option<String>,
    metadata: Option<serde_json::Value>,
    created_at: chrono::DateTime<chrono::Utc>,
    score: f32,
}

// ---------------------------------------------------------------------------
// Text-based fallback search
// ---------------------------------------------------------------------------

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
                      mem_type, tool_name, doc_type, project_path, metadata, created_at
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
                      mem_type, tool_name, doc_type, project_path, metadata, created_at
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
                  mem_type, tool_name, doc_type, project_path, metadata, created_at
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
                  mem_type, tool_name, doc_type, project_path, metadata, created_at
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
