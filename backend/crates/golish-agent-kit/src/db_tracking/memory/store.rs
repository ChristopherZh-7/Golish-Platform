//! Storage helpers: persist user/agent/tool observations to the `memories`
//! table, optionally generating an embedding vector for semantic search.

use super::super::helpers::{await_db_ready, vec_to_pgvector};
use super::super::DbTracker;

impl DbTracker {
    pub fn store_memory(&self, content: &str, mem_type: &str, metadata: Option<serde_json::Value>) {
        let backend = self.backend.clone();
        let session_uuid = self.session_uuid();
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
                    Ok(v) => Some(vec_to_pgvector(&v)),
                    Err(e) => {
                        tracing::warn!(
                            "[db-track] Embedding generation failed, storing text-only: {e}"
                        );
                        None
                    }
                }
            } else {
                None
            };

            backend
                .store_memory(
                    session_uuid,
                    &content,
                    &mem_type,
                    "memory",
                    project_path.as_deref(),
                    metadata.as_ref(),
                    embedding.as_deref(),
                )
                .await;
        });
    }

    pub fn store_memory_global(
        &self,
        content: &str,
        mem_type: &str,
        metadata: Option<serde_json::Value>,
    ) {
        let backend = self.backend.clone();
        let session_uuid = self.session_uuid();
        let content = content.to_string();
        let mem_type = mem_type.to_string();
        let mut gate = self.ready_gate.clone();
        let embedder = self.embedder.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }

            let embedding = if let Some(ref emb) = embedder {
                emb.embed(&content).await.ok().map(|v| vec_to_pgvector(&v))
            } else {
                None
            };

            backend
                .store_memory(
                    session_uuid,
                    &content,
                    &mem_type,
                    "memory",
                    None,
                    metadata.as_ref(),
                    embedding.as_deref(),
                )
                .await;
        });
    }

    pub fn store_memory_with_doc_type(
        &self,
        content: &str,
        mem_type: &str,
        doc_type: &str,
        metadata: Option<serde_json::Value>,
    ) {
        let backend = self.backend.clone();
        let session_uuid = self.session_uuid();
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
                emb.embed(&content).await.ok().map(|v| vec_to_pgvector(&v))
            } else {
                None
            };

            backend
                .store_memory(
                    session_uuid,
                    &content,
                    &mem_type,
                    &doc_type,
                    project_path.as_deref(),
                    metadata.as_ref(),
                    embedding.as_deref(),
                )
                .await;
        });
    }

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
            &content[..{
                let max = content.len().min(200);
                let mut end = max;
                while end > 0 && !content.is_char_boundary(end) {
                    end -= 1;
                }
                end
            }],
        );

        let backend = self.backend.clone();
        let session_uuid = self.session_uuid();
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
            backend
                .store_memory_with_tool(
                    session_uuid,
                    &content,
                    &mem_type,
                    tool_name.as_deref(),
                    project_path.as_deref(),
                    metadata.as_ref(),
                    &emb_str,
                )
                .await;
        });
    }

    pub fn maybe_store_tool_memory(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        result_value: &serde_json::Value,
        success: bool,
    ) {
        use crate::db_traits::{self, StoreDecision, ToolcallStatus};

        let status = if success {
            ToolcallStatus::Finished
        } else {
            ToolcallStatus::Failed
        };

        let decision = db_traits::should_store(tool_name, status);
        let mem_type = match decision {
            StoreDecision::Skip => return,
            StoreDecision::Store(t) | StoreDecision::StoreSummary(t) => t,
        };

        let result_text = match result_value {
            serde_json::Value::String(s) => s.clone(),
            _ => serde_json::to_string(result_value).unwrap_or_default(),
        };

        let filtered = match db_traits::filter_content(&result_text) {
            Some(c) => c,
            None => return,
        };

        let memory_content = db_traits::build_memory_content(tool_name, args, &filtered);

        let mem_type_str = format!("{:?}", mem_type).to_lowercase();
        let metadata = serde_json::json!({
            "tool_name": tool_name,
            "success": success,
        });

        self.store_memory(&memory_content, &mem_type_str, Some(metadata));
    }
}
