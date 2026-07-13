//! Telemetry "record_*" domain methods for `PgTrackingBackend` (inherent
//! `_impl` layer). Bodies moved verbatim from the original `tracking_bridge.rs`
//! trait impl; the trait methods in `mod.rs` delegate here.

use uuid::Uuid;

use golish_agent_kit::db_traits::RuntimeToolIdentity;

use super::PgTrackingBackend;

fn runtime_tool_identity_to_db(
    identity: &RuntimeToolIdentity,
) -> golish_db::repo::tool_calls::RuntimeToolIdentity {
    golish_db::repo::tool_calls::RuntimeToolIdentity {
        operation_id: identity.operation_id,
        stage_execution_id: identity.stage_execution_id,
        stage_run_unit_id: identity.stage_run_unit_id,
        worker_run_id: identity.worker_run_id,
        organization_id: identity.organization_id,
        attempt_epoch: identity.attempt_epoch,
        lease_token: identity.lease_token,
    }
}

impl PgTrackingBackend {
    pub(super) async fn record_tool_call_start_impl(
        &self,
        call_id: &str,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        tool_name: &str,
        args: &serde_json::Value,
        runtime: Option<&RuntimeToolIdentity>,
    ) -> anyhow::Result<Uuid> {
        let runtime = runtime.map(runtime_tool_identity_to_db);
        golish_db::repo::tool_calls::record_tracked_start(
            self.pool.as_ref(),
            call_id,
            session_id,
            task_id,
            subtask_id,
            tool_name,
            args,
            runtime.as_ref(),
        )
        .await
        .map_err(Into::into)
    }

    pub(super) async fn record_tool_call_finish_impl(
        &self,
        record_id: Uuid,
        session_id: Uuid,
        status: &str,
        result: &str,
        duration_ms: i32,
    ) -> anyhow::Result<()> {
        golish_db::repo::tool_calls::record_tracked_finish(
            self.pool.as_ref(),
            record_id,
            session_id,
            status,
            result,
            duration_ms,
        )
        .await
        .map_err(Into::into)
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use golish_agent_kit::db_tracking::DbTracker;
    use golish_agent_kit::db_traits::{DbReadinessGate, RuntimeToolIdentity};
    use sqlx::postgres::PgPoolOptions;
    use uuid::Uuid;

    use super::{runtime_tool_identity_to_db, PgTrackingBackend};

    #[derive(Clone)]
    struct AlwaysReady;

    #[async_trait]
    impl DbReadinessGate for AlwaysReady {
        fn is_ready(&self) -> bool {
            true
        }

        fn is_failed(&self) -> bool {
            false
        }

        async fn wait(&mut self) -> bool {
            true
        }

        fn clone_box(&self) -> Box<dyn DbReadinessGate> {
            Box::new(self.clone())
        }
    }

    #[test]
    fn runtime_tool_tracking_bridge_preserves_every_identity_and_fence_field() {
        let identity = RuntimeToolIdentity {
            operation_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            stage_run_unit_id: Some(Uuid::new_v4()),
            worker_run_id: Some(Uuid::new_v4()),
            organization_id: Some(Uuid::new_v4()),
            attempt_epoch: Some(9),
            lease_token: Some(Uuid::new_v4()),
        };
        let row = runtime_tool_identity_to_db(&identity);
        assert_eq!(row.operation_id, identity.operation_id);
        assert_eq!(row.stage_execution_id, identity.stage_execution_id);
        assert_eq!(row.stage_run_unit_id, identity.stage_run_unit_id);
        assert_eq!(row.worker_run_id, identity.worker_run_id);
        assert_eq!(row.organization_id, identity.organization_id);
        assert_eq!(row.attempt_epoch, identity.attempt_epoch);
        assert_eq!(row.lease_token, identity.lease_token);
    }

    #[tokio::test]
    async fn runtime_tool_tracking_insert_failure_propagates_but_legacy_wrapper_stays_best_effort()
    {
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(25))
            .connect_lazy("postgres://golish:golish@127.0.0.1:1/unavailable")
            .expect("construct lazy unavailable pool");
        let backend = Arc::new(PgTrackingBackend::new(Arc::new(pool)));
        let tracker = DbTracker::new(backend, Uuid::new_v4(), AlwaysReady);
        let identity = RuntimeToolIdentity {
            operation_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            stage_run_unit_id: None,
            worker_run_id: None,
            organization_id: None,
            attempt_epoch: None,
            lease_token: None,
        };

        let strict = tracker
            .start_tool_call_with_runtime(
                "runtime-call",
                "query_target_data",
                &serde_json::json!({}),
                Some(&identity),
            )
            .await;
        assert!(
            strict.is_err(),
            "runtime-aware start must propagate DB failure"
        );

        let legacy = tracker
            .start_tool_call("legacy-call", "read_file", &serde_json::json!({}))
            .await;
        assert_eq!(legacy.record_id, None);
    }
}
