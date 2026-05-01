//! Fire-and-forget DB recording methods: tool calls, token usage,
//! terminal output, search logs, audit entries, agent calls, message logs,
//! and vector store operation logs.

use super::helpers::{await_db_ready, truncate_for_db};
use super::types::ToolCallGuard;
use super::DbTracker;
use std::time::Instant;

impl DbTracker {
    // -- Tool calls --------------------------------------------------------

    pub fn start_tool_call(
        &self,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> ToolCallGuard {
        let backend = self.backend.clone();
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
            backend
                .record_tool_call_start(&call_id_owned, session_uuid, &tool_name, &args)
                .await;
        });

        ToolCallGuard {
            session_uuid: self.session_uuid,
            call_id: call_id_for_guard,
            started_at: Instant::now(),
        }
    }

    pub fn finish_tool_call(&self, guard: ToolCallGuard, success: bool, result_text: &str) {
        let backend = self.backend.clone();
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
            backend
                .record_tool_call_finish(&call_id, session_uuid, status, &result_text, duration)
                .await;
        });
    }

    // -- Token usage / message chains --------------------------------------

    pub fn record_token_usage(
        &self,
        tokens_in: u64,
        tokens_out: u64,
        model: &str,
        provider: &str,
        duration_ms: u64,
    ) {
        let backend = self.backend.clone();
        let session_uuid = self.session_uuid;
        let model = model.to_string();
        let provider = provider.to_string();
        let mut gate = self.ready_gate.clone();

        tokio::spawn(async move {
            if !await_db_ready(&mut gate).await {
                return;
            }
            backend
                .record_token_usage(
                    session_uuid,
                    &model,
                    &provider,
                    tokens_in as i32,
                    tokens_out as i32,
                    duration_ms as i32,
                )
                .await;
        });
    }

    // -- Terminal logs -----------------------------------------------------

    pub fn record_terminal_output(&self, stream: &str, content: &str) {
        let backend = self.backend.clone();
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
            backend
                .record_terminal_output(session_uuid, task_id, subtask_id, &stream, &content, &pp)
                .await;
        });
    }

    // -- Search logs -------------------------------------------------------

    pub fn record_search(&self, engine: &str, query: &str, result: Option<&str>) {
        let backend = self.backend.clone();
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
            backend
                .record_search_log(
                    session_uuid,
                    task_id,
                    subtask_id,
                    &engine,
                    &query,
                    result.as_deref(),
                    &pp,
                )
                .await;
        });
    }

    // -- Audit log ---------------------------------------------------------

    pub fn audit(&self, action: &str, category: &str, details: &str) {
        self.audit_with_source(action, category, details, "ai");
    }

    pub fn audit_with_source(&self, action: &str, category: &str, details: &str, source: &str) {
        let backend = self.backend.clone();
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
            backend
                .record_audit(&action, &category, &details, &source, &session_id, pp.as_deref())
                .await;
        });
    }

    // -- Agent / message / vecstore logs -----------------------------------

    pub fn record_agent_call(
        &self,
        initiator: &str,
        executor: &str,
        task: &str,
        result: Option<&str>,
        duration_ms: u64,
    ) {
        let backend = self.backend.clone();
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
            backend
                .record_agent_call(
                    session_uuid,
                    &initiator,
                    &executor,
                    &task,
                    result.as_deref(),
                    duration_ms,
                    &pp,
                )
                .await;
        });
    }

    pub fn record_msg_log(
        &self,
        msg_type: &str,
        agent: &str,
        message: &str,
        thinking: Option<&str>,
    ) {
        let backend = self.backend.clone();
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
            backend
                .record_msg_log(
                    session_uuid,
                    task_id,
                    subtask_id,
                    &agent,
                    &msg_type,
                    &message,
                    thinking.as_deref(),
                    pp.as_deref(),
                )
                .await;
        });
    }

    pub fn record_vecstore_op(
        &self,
        action: &str,
        query: &str,
        result_count: i32,
        result_preview: &str,
    ) {
        let backend = self.backend.clone();
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
            backend
                .record_vecstore_op(
                    session_uuid,
                    task_id,
                    subtask_id,
                    &action,
                    &query,
                    &result_preview,
                    result_count,
                    pp.as_deref(),
                )
                .await;
        });
    }
}
