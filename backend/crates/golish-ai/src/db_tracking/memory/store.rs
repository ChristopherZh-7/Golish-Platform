//! Storage helpers: persist user/agent/tool observations to the `memories`
//! table, optionally generating an embedding vector for semantic search.

use super::super::helpers::{await_db_ready, vec_to_pgvector};
use super::super::DbTracker;

impl DbTracker {
    /// Store a memory observation. When an embedder is configured, generates and
    /// stores an embedding vector alongside the text for semantic search.
    pub fn store_memory(&self, content: &str, mem_type: &str, metadata: Option<serde_json::Value>) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let content = content.to_string();
        let mem_type = mem_type.to_string();
        let project_path = self.project_path.clone();
        let mut gate = self.ready_gate.clone();
        let embedder = self.embedder.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }

            let embedding = if let Some(ref emb) = embedder {
                match emb.embed(&content).await {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::warn!("[db-track] Embedding generation failed, storing text-only: {e}");
                        None
                    }
                }
            } else {
                None
            };

            let res = if let Some(ref emb_vec) = embedding {
                let emb_str = vec_to_pgvector(emb_vec);
                sqlx::query(
                    r#"INSERT INTO memories
                       (session_id, content, mem_type, doc_type, project_path, metadata, embedding)
                       VALUES ($1, $2, $3::memory_type, 'memory', $4, $5, $6::vector)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&project_path)
                .bind(&metadata)
                .bind(&emb_str)
                .execute(pool.as_ref())
                .await
            } else {
                sqlx::query(
                    r#"INSERT INTO memories (session_id, content, mem_type, doc_type, project_path, metadata)
                       VALUES ($1, $2, $3::memory_type, 'memory', $4, $5)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&project_path)
                .bind(&metadata)
                .execute(pool.as_ref())
                .await
            };

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to store memory: {e}");
            }
        });
    }

    /// Store a global memory (project_path = NULL), visible across all projects.
    /// Use for general techniques, tool usage patterns, and reusable knowledge.
    pub fn store_memory_global(
        &self,
        content: &str,
        mem_type: &str,
        metadata: Option<serde_json::Value>,
    ) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let content = content.to_string();
        let mem_type = mem_type.to_string();
        let mut gate = self.ready_gate.clone();
        let embedder = self.embedder.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }

            let embedding = if let Some(ref emb) = embedder {
                emb.embed(&content).await.ok()
            } else {
                None
            };

            let res = if let Some(ref emb_vec) = embedding {
                let emb_str = vec_to_pgvector(emb_vec);
                sqlx::query(
                    r#"INSERT INTO memories
                       (session_id, content, mem_type, doc_type, project_path, metadata, embedding)
                       VALUES ($1, $2, $3::memory_type, 'memory', NULL, $4, $5::vector)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&metadata)
                .bind(&emb_str)
                .execute(pool.as_ref())
                .await
            } else {
                sqlx::query(
                    r#"INSERT INTO memories (session_id, content, mem_type, doc_type, project_path, metadata)
                       VALUES ($1, $2, $3::memory_type, 'memory', NULL, $4)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&metadata)
                .execute(pool.as_ref())
                .await
            };

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to store global memory: {e}");
            }
        });
    }

    /// Store a memory with a specific `doc_type` (for multi-vector store: "code", "guide").
    pub fn store_memory_with_doc_type(
        &self,
        content: &str,
        mem_type: &str,
        doc_type: &str,
        metadata: Option<serde_json::Value>,
    ) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let content = content.to_string();
        let mem_type = mem_type.to_string();
        let doc_type = doc_type.to_string();
        let project_path = self.project_path.clone();
        let mut gate = self.ready_gate.clone();
        let embedder = self.embedder.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }

            let embedding = if let Some(ref emb) = embedder {
                emb.embed(&content).await.ok()
            } else {
                None
            };

            let res = if let Some(ref emb_vec) = embedding {
                let emb_str = vec_to_pgvector(emb_vec);
                sqlx::query(
                    r#"INSERT INTO memories
                       (session_id, content, mem_type, doc_type, project_path, metadata, embedding)
                       VALUES ($1, $2, $3::memory_type, $4, $5, $6, $7::vector)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&doc_type)
                .bind(&project_path)
                .bind(&metadata)
                .bind(&emb_str)
                .execute(pool.as_ref())
                .await
            } else {
                sqlx::query(
                    r#"INSERT INTO memories (session_id, content, mem_type, doc_type, project_path, metadata)
                       VALUES ($1, $2, $3::memory_type, $4, $5, $6)"#,
                )
                .bind(session_uuid)
                .bind(&content)
                .bind(&mem_type)
                .bind(&doc_type)
                .bind(&project_path)
                .bind(&metadata)
                .execute(pool.as_ref())
                .await
            };

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to store {} memory: {e}", doc_type);
            }
        });
    }

    /// Store a memory with an embedding vector for semantic search.
    /// Adapts to pgvector (native vector type) or BYTEA fallback automatically.
    pub fn store_memory_with_embedding(
        &self,
        content: &str,
        mem_type: &str,
        embedding: Vec<f32>,
        tool_name: Option<&str>,
        metadata: Option<serde_json::Value>,
    ) {
        self.record_vecstore_op(
            "store",
            &format!("{}:{}", mem_type, tool_name.unwrap_or("unknown")),
            1,
            &content[..content.len().min(200)],
        );

        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let content = content.to_string();
        let mem_type = mem_type.to_string();
        let tool_name = tool_name.map(str::to_string);
        let project_path = self.project_path.clone();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }

            let emb_str = vec_to_pgvector(&embedding);
            let res = sqlx::query(
                r#"INSERT INTO memories
                   (session_id, content, mem_type, doc_type, tool_name,
                    embedding, project_path, metadata)
                   VALUES ($1, $2, $3::memory_type, 'tool_result', $4, $5::vector, $6, $7)"#,
            )
            .bind(session_uuid)
            .bind(&content)
            .bind(&mem_type)
            .bind(&tool_name)
            .bind(&emb_str)
            .bind(&project_path)
            .bind(&metadata)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to store memory with embedding: {e}");
            }
        });
    }

    /// Run the gatekeeper pipeline: decide whether to store a tool result as
    /// a long-term memory, build the structured content, and persist it.
    /// This is a fire-and-forget background task.
    pub fn maybe_store_tool_memory(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        result_value: &serde_json::Value,
        success: bool,
    ) {
        use golish_db::gatekeeper::{self, StoreDecision};
        use golish_db::models::ToolcallStatus;

        let status = if success {
            ToolcallStatus::Finished
        } else {
            ToolcallStatus::Failed
        };

        let decision = gatekeeper::should_store(tool_name, status);
        let mem_type = match decision {
            StoreDecision::Skip => return,
            StoreDecision::Store(t) | StoreDecision::StoreSummary(t) => t,
        };

        let result_text = match result_value {
            serde_json::Value::String(s) => s.clone(),
            _ => serde_json::to_string(result_value).unwrap_or_default(),
        };

        let filtered = match gatekeeper::filter_content(&result_text) {
            Some(c) => c,
            None => return,
        };

        let memory_content = gatekeeper::build_memory_content(tool_name, args, &filtered);

        let mem_type_str = format!("{:?}", mem_type).to_lowercase();
        let metadata = serde_json::json!({
            "tool_name": tool_name,
            "success": success,
        });

        self.store_memory(&memory_content, &mem_type_str, Some(metadata));
    }
}
