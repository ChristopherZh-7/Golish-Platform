//! Fire-and-forget DB recording methods: tool calls, token usage,
//! terminal output, search logs, audit entries, agent calls, message logs,
//! and vector store operation logs.

use super::helpers::{await_db_ready, truncate_for_db};
use super::types::ToolCallGuard;
use super::DbTracker;
use crate::db_traits::RuntimeToolIdentity;
use std::time::Instant;

fn canonical_runtime_task_owner(
    tracker_task_id: Option<uuid::Uuid>,
    runtime: Option<&RuntimeToolIdentity>,
) -> anyhow::Result<Option<uuid::Uuid>> {
    let Some(runtime) = runtime else {
        return Ok(tracker_task_id);
    };

    if let Some(tracker_task_id) = tracker_task_id {
        anyhow::ensure!(
            tracker_task_id == runtime.operation_id,
            "runtime tool-call owner mismatch: tracker task_id {tracker_task_id} does not match operation_id {}",
            runtime.operation_id
        );
    }

    Ok(Some(runtime.operation_id))
}

impl DbTracker {
    // -- Tool calls --------------------------------------------------------

    pub async fn start_tool_call(
        &self,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> ToolCallGuard {
        match self
            .start_tool_call_with_runtime(call_id, tool_name, args, None)
            .await
        {
            Ok(guard) => guard,
            Err(error) => {
                tracing::warn!(
                    call_id,
                    tool_name,
                    %error,
                    "[db-track] legacy tool_call_start failed; continuing without telemetry"
                );
                ToolCallGuard {
                    record_id: None,
                    session_uuid: self.session_uuid(),
                    call_id: call_id.to_string(),
                    started_at: Instant::now(),
                }
            }
        }
    }

    /// Await the durable start row and optionally stamp a trusted runtime
    /// identity. Runtime-aware callers receive readiness/insert failures and
    /// must stop before tool dispatch; the legacy wrapper above keeps ordinary
    /// chat telemetry best-effort.
    pub async fn start_tool_call_with_runtime(
        &self,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
        runtime: Option<&RuntimeToolIdentity>,
    ) -> anyhow::Result<ToolCallGuard> {
        let session_uuid = self.session_uuid();
        let task_id = canonical_runtime_task_owner(self.task_id, runtime)?;
        let mut gate = self.ready_gate.clone();
        if !await_db_ready(&mut gate).await {
            if runtime.is_some() {
                anyhow::bail!("runtime tool-call tracking database is unavailable");
            }
            return Ok(ToolCallGuard {
                record_id: None,
                session_uuid,
                call_id: call_id.to_string(),
                started_at: Instant::now(),
            });
        }

        let record_id = self
            .backend
            .record_tool_call_start(
                call_id,
                session_uuid,
                task_id,
                self.subtask_id,
                tool_name,
                args,
                runtime,
            )
            .await?;
        Ok(ToolCallGuard {
            record_id: Some(record_id),
            session_uuid,
            call_id: call_id.to_string(),
            started_at: Instant::now(),
        })
    }

    pub async fn finish_tool_call(&self, guard: ToolCallGuard, success: bool, result_text: &str) {
        if let Err(error) = self
            .finish_tool_call_checked(guard, success, result_text)
            .await
        {
            tracing::warn!(%error, "[db-track] tool_call_finish failed");
        }
    }

    /// Finish by the persisted DB primary key and the session UUID captured at
    /// start. This is the strict ordered seam for runtime-aware callers; the
    /// legacy wrapper keeps its historical best-effort behavior.
    pub async fn finish_tool_call_checked(
        &self,
        guard: ToolCallGuard,
        success: bool,
        result_text: &str,
    ) -> anyhow::Result<()> {
        let Some(record_id) = guard.record_id else {
            return Ok(());
        };
        let session_uuid = guard.session_uuid;
        let call_id = guard.call_id;
        let duration = guard.started_at.elapsed().as_millis() as i32;
        let status = if success { "finished" } else { "failed" };
        let result_text = truncate_for_db(result_text, 50_000);
        let mut gate = self.ready_gate.clone();
        if !await_db_ready(&mut gate).await {
            anyhow::bail!("tool-call finish database is unavailable: call_id={call_id}");
        }
        self.backend
            .record_tool_call_finish(record_id, session_uuid, status, &result_text, duration)
            .await
            .map_err(|error| anyhow::anyhow!("finish tool_call {call_id}: {error}"))
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
        let session_uuid = self.session_uuid();
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
        let session_uuid = self.session_uuid();
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
        let session_uuid = self.session_uuid();
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
        let session_id = self.session_uuid().to_string();
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
                .record_audit(
                    &action,
                    &category,
                    &details,
                    &source,
                    &session_id,
                    pp.as_deref(),
                )
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
        let session_uuid = self.session_uuid();
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
        let session_uuid = self.session_uuid();
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
        let session_uuid = self.session_uuid();
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

#[cfg(test)]
mod runtime_task_owner_tests {
    use super::canonical_runtime_task_owner;
    use crate::db_traits::RuntimeToolIdentity;
    use uuid::Uuid;

    fn runtime_identity(operation_id: Uuid) -> RuntimeToolIdentity {
        RuntimeToolIdentity {
            operation_id,
            stage_execution_id: Uuid::new_v4(),
            stage_run_unit_id: None,
            worker_run_id: None,
            organization_id: None,
            attempt_epoch: None,
            lease_token: None,
        }
    }

    #[test]
    fn runtime_identity_supplies_task_owner_when_tracker_is_unbound() {
        let operation_id = Uuid::new_v4();
        let runtime = runtime_identity(operation_id);

        assert_eq!(
            canonical_runtime_task_owner(None, Some(&runtime)).unwrap(),
            Some(operation_id)
        );
    }

    #[test]
    fn runtime_identity_accepts_the_same_tracker_task_owner() {
        let operation_id = Uuid::new_v4();
        let runtime = runtime_identity(operation_id);

        assert_eq!(
            canonical_runtime_task_owner(Some(operation_id), Some(&runtime)).unwrap(),
            Some(operation_id)
        );
    }

    #[test]
    fn runtime_identity_rejects_a_different_tracker_task_owner() {
        let operation_id = Uuid::new_v4();
        let tracker_task_id = Uuid::new_v4();
        let runtime = runtime_identity(operation_id);

        let error = canonical_runtime_task_owner(Some(tracker_task_id), Some(&runtime))
            .expect_err("mismatched runtime owner must fail closed");

        assert!(error
            .to_string()
            .contains("runtime tool-call owner mismatch"));
    }

    #[test]
    fn legacy_tracking_preserves_the_tracker_task_owner() {
        let tracker_task_id = Uuid::new_v4();

        assert_eq!(
            canonical_runtime_task_owner(Some(tracker_task_id), None).unwrap(),
            Some(tracker_task_id)
        );
    }
}
