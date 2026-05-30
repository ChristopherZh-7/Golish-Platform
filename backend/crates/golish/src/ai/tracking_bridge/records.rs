//! Telemetry "record_*" domain methods for `PgTrackingBackend` (inherent
//! `_impl` layer). Bodies moved verbatim from the original `tracking_bridge.rs`
//! trait impl; the trait methods in `mod.rs` delegate here.

use uuid::Uuid;

use super::PgTrackingBackend;

impl PgTrackingBackend {
    pub(super) async fn record_tool_call_start_impl(
        &self,
        call_id: &str,
        session_id: Uuid,
        tool_name: &str,
        args: &serde_json::Value,
    ) {
        let res = sqlx::query(
            r#"INSERT INTO tool_calls (call_id, session_id, agent, name, args, status, source)
               VALUES ($1, $2, 'primary'::agent_type, $3, $4, 'running'::toolcall_status, 'ai')
               ON CONFLICT DO NOTHING"#,
        )
        .bind(call_id)
        .bind(session_id)
        .bind(tool_name)
        .bind(args)
        .execute(self.pool.as_ref())
        .await;
        if let Err(e) = res {
            tracing::warn!("[db-track] tool_call_start: {e}");
        }
    }

    pub(super) async fn record_tool_call_finish_impl(
        &self,
        call_id: &str,
        session_id: Uuid,
        status: &str,
        result: &str,
        duration_ms: i32,
    ) {
        let res = sqlx::query(
            r#"UPDATE tool_calls SET status = $1::toolcall_status, result = $2, duration_ms = $3, updated_at = NOW()
               WHERE call_id = $4 AND session_id = $5"#,
        )
        .bind(status).bind(result).bind(duration_ms).bind(call_id).bind(session_id)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res {
            tracing::warn!("[db-track] tool_call_finish: {e}");
        }
    }

    pub(super) async fn record_token_usage_impl(
        &self,
        session_id: Uuid,
        model: &str,
        provider: &str,
        tokens_in: i32,
        tokens_out: i32,
        duration_ms: i32,
    ) {
        let res = sqlx::query(
            r#"INSERT INTO message_chains (session_id, agent, model, provider, tokens_in, tokens_out, duration_ms)
               VALUES ($1, 'primary'::agent_type, $2, $3, $4, $5, $6)"#,
        )
        .bind(session_id).bind(model).bind(provider).bind(tokens_in).bind(tokens_out).bind(duration_ms)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res {
            tracing::warn!("[db-track] token_usage: {e}");
        }
    }

    pub(super) async fn record_terminal_output_impl(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        stream: &str,
        content: &str,
        project_path: &str,
    ) {
        let res = sqlx::query(
            r#"INSERT INTO terminal_logs (session_id, task_id, subtask_id, stream, content, project_path)
               VALUES ($1, $2, $3, $4::stream_type, $5, $6)"#,
        )
        .bind(session_id).bind(task_id).bind(subtask_id).bind(stream).bind(content).bind(project_path)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res {
            tracing::warn!("[db-track] terminal_output: {e}");
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn record_search_log_impl(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        engine: &str,
        query: &str,
        result: Option<&str>,
        project_path: &str,
    ) {
        let res = sqlx::query(
            r#"INSERT INTO search_logs (session_id, task_id, subtask_id, initiator, engine, query, result, project_path)
               VALUES ($1, $2, $3, 'primary'::agent_type, $4, $5, $6, $7)"#,
        )
        .bind(session_id).bind(task_id).bind(subtask_id).bind(engine).bind(query).bind(result).bind(project_path)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res {
            tracing::warn!("[db-track] search_log: {e}");
        }
    }

    pub(super) async fn record_audit_impl(
        &self,
        action: &str,
        category: &str,
        details: &str,
        source: &str,
        session_id_str: &str,
        project_path: Option<&str>,
    ) {
        let res = sqlx::query(
            r#"INSERT INTO audit_log (action, category, details, source, session_id, project_path)
               VALUES ($1, $2, $3, $4, $5, $6)"#,
        )
        .bind(action)
        .bind(category)
        .bind(details)
        .bind(source)
        .bind(session_id_str)
        .bind(project_path)
        .execute(self.pool.as_ref())
        .await;
        if let Err(e) = res {
            tracing::warn!("[db-track] audit: {e}");
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn record_agent_call_impl(
        &self,
        session_id: Uuid,
        initiator: &str,
        executor: &str,
        task: &str,
        result: Option<&str>,
        duration_ms: i32,
        project_path: &str,
    ) {
        let res = sqlx::query(
            r#"INSERT INTO agent_logs (session_id, initiator, executor, task, result, duration_ms, project_path)
               VALUES ($1, $2::agent_type, $3::agent_type, $4, $5, $6, $7)"#,
        )
        .bind(session_id).bind(initiator).bind(executor).bind(task).bind(result).bind(duration_ms).bind(project_path)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res {
            tracing::warn!("[db-track] agent_call: {e}");
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn record_msg_log_impl(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        agent: &str,
        msg_type: &str,
        message: &str,
        thinking: Option<&str>,
        project_path: Option<&str>,
    ) {
        let res = sqlx::query(
            r#"INSERT INTO msg_logs (session_id, task_id, subtask_id, agent, msg_type, message, thinking, project_path)
               VALUES ($1, $2, $3, $4::agent_type, $5::msglog_type, $6, $7, $8)"#,
        )
        .bind(session_id).bind(task_id).bind(subtask_id).bind(agent).bind(msg_type).bind(message).bind(thinking).bind(project_path)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res {
            tracing::warn!("[db-track] msg_log: {e}");
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn record_vecstore_op_impl(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        action: &str,
        query: &str,
        result_preview: &str,
        result_count: i32,
        project_path: Option<&str>,
    ) {
        let res = sqlx::query(
            r#"INSERT INTO vector_store_logs (session_id, task_id, subtask_id, action, query, result, result_count, project_path)
               VALUES ($1, $2, $3, $4::vecstore_action, $5, $6, $7, $8)"#,
        )
        .bind(session_id).bind(task_id).bind(subtask_id).bind(action).bind(query).bind(result_preview).bind(result_count).bind(project_path)
        .execute(self.pool.as_ref()).await;
        if let Err(e) = res {
            tracing::warn!("[db-track] vecstore_op: {e}");
        }
    }
}
