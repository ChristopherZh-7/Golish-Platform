//! Fire-and-forget DB recording methods: tool calls, token usage,
//! terminal output, search logs, audit entries, agent calls, message logs,
//! and vector store operation logs.

use super::helpers::{await_db_ready, truncate_for_db};
use super::types::ToolCallGuard;
use super::DbTracker;
use std::time::Instant;

impl DbTracker {
    // -- Tool calls --------------------------------------------------------

    /// Record a tool call start. Returns a guard with a start timestamp so
    /// `finish_tool_call` can compute duration.
    pub fn start_tool_call(
        &self,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> ToolCallGuard {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let call_id_owned = call_id.to_string();
        let tool_name = tool_name.to_string();
        let args = args.clone();
        let mut gate = self.ready_gate.clone();

        let call_id_for_guard = call_id_owned.clone();
        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO tool_calls (call_id, session_id, agent, name, args, status, source)
                   VALUES ($1, $2, 'primary'::agent_type, $3, $4, 'running'::toolcall_status, 'ai')
                   ON CONFLICT DO NOTHING"#,
            )
            .bind(&call_id_owned)
            .bind(session_uuid)
            .bind(&tool_name)
            .bind(&args)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record tool call start: {e}");
            }
        });

        ToolCallGuard {
            pool: self.pool.clone(),
            session_uuid: self.session_uuid,
            call_id: call_id_for_guard,
            started_at: Instant::now(),
        }
    }

    /// Record a completed tool call result.
    pub fn finish_tool_call(&self, guard: ToolCallGuard, success: bool, result_text: &str) {
        let pool = guard.pool;
        let session_uuid = guard.session_uuid;
        let call_id = guard.call_id;
        let duration = guard.started_at.elapsed().as_millis() as i32;
        let status = if success { "finished" } else { "failed" };
        let result_text = truncate_for_db(result_text, 50_000);
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"UPDATE tool_calls
                   SET status = $1::toolcall_status, result = $2,
                       duration_ms = $3, updated_at = NOW()
                   WHERE call_id = $4 AND session_id = $5"#,
            )
            .bind(status)
            .bind(&result_text)
            .bind(duration)
            .bind(&call_id)
            .bind(session_uuid)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record tool call finish: {e}");
            }
        });
    }

    // -- Token usage / message chains --------------------------------------

    /// Record token usage for a single LLM turn.
    pub fn record_token_usage(
        &self,
        tokens_in: u64,
        tokens_out: u64,
        model: &str,
        provider: &str,
        duration_ms: u64,
    ) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let model = model.to_string();
        let provider = provider.to_string();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO message_chains
                   (session_id, agent, model, provider, tokens_in, tokens_out, duration_ms)
                   VALUES ($1, 'primary'::agent_type, $2, $3, $4, $5, $6)"#,
            )
            .bind(session_uuid)
            .bind(&model)
            .bind(&provider)
            .bind(tokens_in as i32)
            .bind(tokens_out as i32)
            .bind(duration_ms as i32)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record token usage: {e}");
            }
        });
    }

    // -- Terminal logs -----------------------------------------------------

    /// Record terminal output (stdout/stderr) from a command execution.
    pub fn record_terminal_output(&self, stream: &str, content: &str) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let task_id = self.task_id;
        let subtask_id = self.subtask_id;
        let stream = stream.to_string();
        let content = truncate_for_db(content, 100_000);
        let pp = self.project_path.clone().unwrap_or_default();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO terminal_logs (session_id, task_id, subtask_id, stream, content, project_path)
                   VALUES ($1, $2, $3, $4::stream_type, $5, $6)"#,
            )
            .bind(session_uuid)
            .bind(task_id)
            .bind(subtask_id)
            .bind(&stream)
            .bind(&content)
            .bind(&pp)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record terminal output: {e}");
            }
        });
    }

    // -- Search logs -------------------------------------------------------

    /// Record a web search query and its result.
    pub fn record_search(&self, engine: &str, query: &str, result: Option<&str>) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let task_id = self.task_id;
        let subtask_id = self.subtask_id;
        let engine = engine.to_string();
        let query = query.to_string();
        let result = result.map(|r| truncate_for_db(r, 50_000));
        let pp = self.project_path.clone().unwrap_or_default();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO search_logs (session_id, task_id, subtask_id, initiator, engine, query, result, project_path)
                   VALUES ($1, $2, $3, 'primary'::agent_type, $4, $5, $6, $7)"#,
            )
            .bind(session_uuid)
            .bind(task_id)
            .bind(subtask_id)
            .bind(&engine)
            .bind(&query)
            .bind(&result)
            .bind(&pp)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record search log: {e}");
            }
        });
    }

    // -- Audit log ---------------------------------------------------------

    /// Record an audit log entry.
    pub fn audit(&self, action: &str, category: &str, details: &str) {
        self.audit_with_source(action, category, details, "ai");
    }

    /// Record an audit log entry with explicit source.
    pub fn audit_with_source(&self, action: &str, category: &str, details: &str, source: &str) {
        let pool = self.pool.clone();
        let session_id = self.session_uuid.to_string();
        let pp = self.project_path.clone();
        let action = action.to_string();
        let category = category.to_string();
        let details = details.to_string();
        let source = source.to_string();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO audit_log (action, category, details, source, session_id, project_path)
                   VALUES ($1, $2, $3, $4, $5, $6)"#,
            )
            .bind(&action)
            .bind(&category)
            .bind(&details)
            .bind(&source)
            .bind(&session_id)
            .bind(&pp)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record audit entry: {e}");
            }
        });
    }

    // -- Agent / message / vecstore logs -----------------------------------

    /// Record a sub-agent execution in the agent_logs table.
    /// Fire-and-forget — errors are logged but don't propagate.
    pub fn record_agent_call(
        &self,
        initiator: &str,
        executor: &str,
        task: &str,
        result: Option<&str>,
        duration_ms: u64,
    ) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let initiator = initiator.to_string();
        let executor = executor.to_string();
        let task = task.to_string();
        let result = result.map(|r| r.to_string());
        let duration_ms = duration_ms as i32;
        let pp = self.project_path.clone().unwrap_or_default();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO agent_logs (session_id, initiator, executor, task, result, duration_ms, project_path)
                   VALUES ($1, $2::agent_type, $3::agent_type, $4, $5, $6, $7)"#,
            )
            .bind(session_uuid)
            .bind(&initiator)
            .bind(&executor)
            .bind(&task)
            .bind(&result)
            .bind(duration_ms)
            .bind(&pp)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record agent call: {e}");
            }
        });
    }

    /// Record an LLM conversation message (PentAGI-style msg_log).
    /// Fire-and-forget — does not block the caller.
    pub fn record_msg_log(
        &self,
        msg_type: &str,
        agent: &str,
        message: &str,
        thinking: Option<&str>,
    ) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let task_id = self.task_id;
        let subtask_id = self.subtask_id;
        let msg_type = msg_type.to_string();
        let agent = agent.to_string();
        let message = message.to_string();
        let thinking = thinking.map(|t| t.to_string());
        let pp = self.project_path.clone();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO msg_logs (session_id, task_id, subtask_id, agent, msg_type, message, thinking, project_path)
                   VALUES ($1, $2, $3, $4::agent_type, $5::msglog_type, $6, $7, $8)"#,
            )
            .bind(session_uuid)
            .bind(task_id)
            .bind(subtask_id)
            .bind(&agent)
            .bind(&msg_type)
            .bind(&message)
            .bind(&thinking)
            .bind(&pp)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record msg_log: {e}");
            }
        });
    }

    /// Record a vector store operation (store/search/delete) for audit trail.
    pub fn record_vecstore_op(
        &self,
        action: &str,
        query: &str,
        result_count: i32,
        result_preview: &str,
    ) {
        let pool = self.pool.clone();
        let session_uuid = self.session_uuid;
        let task_id = self.task_id;
        let subtask_id = self.subtask_id;
        let action = action.to_string();
        let query = query.to_string();
        let result_preview = result_preview.to_string();
        let pp = self.project_path.clone();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            let res = sqlx::query(
                r#"INSERT INTO vector_store_logs (session_id, task_id, subtask_id, action, query, result, result_count, project_path)
                   VALUES ($1, $2, $3, $4::vecstore_action, $5, $6, $7, $8)"#,
            )
            .bind(session_uuid)
            .bind(task_id)
            .bind(subtask_id)
            .bind(&action)
            .bind(&query)
            .bind(&result_preview)
            .bind(result_count)
            .bind(&pp)
            .execute(pool.as_ref())
            .await;

            if let Err(e) = res {
                tracing::warn!("[db-track] Failed to record vecstore log: {e}");
            }
        });
    }
}
