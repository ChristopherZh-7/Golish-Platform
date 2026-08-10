//! Durable tool-call row + V2 worker active-tool fencing.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::db_tracking::{DbTracker, ToolCallGuard};
use golish_agent_kit::db_traits::{
    RuntimeMemoryRepository, RuntimeToolIdentity, RuntimeWorkerFence, WorkerToolMutation,
};
use golish_sub_agents::{
    BoundWorkerChainContext, BoundWorkerNestedDelegationLifecycle, BoundWorkerToolLifecycle,
};
use tokio::sync::Mutex;
use uuid::Uuid;

fn begin_worker_tool_error_invalidates_lease(
    error: &golish_agent_kit::db_traits::RuntimeMemoryError,
) -> bool {
    matches!(
        error,
        golish_agent_kit::db_traits::RuntimeMemoryError::LeaseLost { .. }
    )
}

pub(crate) struct RuntimeWorkerToolLifecycle {
    tracker: DbTracker,
    repository: Arc<dyn RuntimeMemoryRepository>,
    bound: BoundWorkerChainContext,
    guards: Mutex<HashMap<Uuid, ToolCallGuard>>,
    nested: Option<Arc<dyn BoundWorkerNestedDelegationLifecycle>>,
}

impl RuntimeWorkerToolLifecycle {
    pub(crate) fn new(
        tracker: DbTracker,
        repository: Arc<dyn RuntimeMemoryRepository>,
        bound: BoundWorkerChainContext,
    ) -> Self {
        Self {
            tracker,
            repository,
            bound,
            guards: Mutex::new(HashMap::new()),
            nested: None,
        }
    }

    pub(crate) fn new_with_nested(
        tracker: DbTracker,
        repository: Arc<dyn RuntimeMemoryRepository>,
        bound: BoundWorkerChainContext,
        nested: Arc<dyn BoundWorkerNestedDelegationLifecycle>,
    ) -> Self {
        Self {
            tracker,
            repository,
            bound,
            guards: Mutex::new(HashMap::new()),
            nested: Some(nested),
        }
    }

    fn fence(&self) -> RuntimeWorkerFence {
        RuntimeWorkerFence {
            operation_id: self.bound.operation_id,
            stage_execution_id: self.bound.stage_execution_id,
            stage_run_unit_id: self.bound.worker_lease.stage_run_unit_id,
            worker_run_id: self.bound.worker_lease.worker_run_id,
            lease_token: self.bound.worker_lease.lease_token,
            attempt_epoch: self.bound.worker_lease.attempt_epoch,
            expected_checkpoint_version: self.bound.current_checkpoint_version(),
        }
    }
}

#[async_trait]
impl BoundWorkerToolLifecycle for RuntimeWorkerToolLifecycle {
    fn nested_delegation_lifecycle(&self) -> Option<Arc<dyn BoundWorkerNestedDelegationLifecycle>> {
        self.nested.clone()
    }

    async fn begin(
        &self,
        request_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> anyhow::Result<Uuid> {
        anyhow::ensure!(!self.bound.lease_is_lost(), "worker lease was already lost");
        let runtime = RuntimeToolIdentity {
            operation_id: self.bound.operation_id,
            stage_execution_id: self.bound.stage_execution_id,
            stage_run_unit_id: Some(self.bound.worker_lease.stage_run_unit_id),
            worker_run_id: Some(self.bound.worker_lease.worker_run_id),
            organization_id: Some(self.bound.organization_id),
            attempt_epoch: Some(self.bound.worker_lease.attempt_epoch),
            lease_token: Some(self.bound.worker_lease.lease_token),
        };
        // The durable generic tool row must exist before the worker advertises
        // an active external side effect.
        let guard = self
            .tracker
            .start_tool_call_with_runtime(request_id, tool_name, args, Some(&runtime))
            .await?;
        let record_id = guard
            .record_id
            .ok_or_else(|| anyhow::anyhow!("durable worker tool start returned no record id"))?;

        let _mutation_guard = self.bound.mutation_lock.lock().await;
        if self.bound.lease_is_lost() {
            self.tracker
                .finish_tool_call_checked(guard, false, "worker lease lost before dispatch")
                .await?;
            anyhow::bail!("worker lease was lost before tool dispatch")
        }
        if let Err(error) = self
            .repository
            .begin_worker_tool(WorkerToolMutation {
                fence: self.fence(),
                tool_call_record_id: record_id,
            })
            .await
        {
            if begin_worker_tool_error_invalidates_lease(&error) {
                self.bound.mark_lease_lost();
            }
            let result = format!("worker tool fence rejected before dispatch: {error}");
            self.tracker
                .finish_tool_call_checked(guard, false, &result)
                .await?;
            return Err(anyhow::anyhow!(result));
        }
        self.guards.lock().await.insert(record_id, guard);
        Ok(record_id)
    }

    async fn finish(
        &self,
        tool_call_record_id: Uuid,
        success: bool,
        result: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let guard = self
            .guards
            .lock()
            .await
            .remove(&tool_call_record_id)
            .ok_or_else(|| anyhow::anyhow!("worker tool start guard is missing"))?;
        let result_text = serde_json::to_string(result).unwrap_or_default();

        let finish_result = {
            let _mutation_guard = self.bound.mutation_lock.lock().await;
            if self.bound.lease_is_lost() {
                Err(anyhow::anyhow!(
                    "worker lease was lost before tool result landing"
                ))
            } else {
                self.repository
                    .finish_worker_tool(WorkerToolMutation {
                        fence: self.fence(),
                        tool_call_record_id,
                    })
                    .await
                    .map(|_| ())
                    .map_err(anyhow::Error::from)
            }
        };
        if let Err(error) = finish_result {
            self.bound.mark_lease_lost();
            let stale = format!("worker tool result rejected by lease fence: {error}");
            self.tracker
                .finish_tool_call_checked(guard, false, &stale)
                .await?;
            return Err(anyhow::anyhow!(stale));
        }

        self.tracker
            .finish_tool_call_checked(guard, success, &result_text)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::begin_worker_tool_error_invalidates_lease;
    use golish_agent_kit::db_traits::RuntimeMemoryError;
    use uuid::Uuid;

    #[test]
    fn begin_worker_tool_error_marks_only_typed_lease_loss() {
        assert!(!begin_worker_tool_error_invalidates_lease(
            &RuntimeMemoryError::Storage("deadlock detected".to_string())
        ));
        assert!(!begin_worker_tool_error_invalidates_lease(
            &RuntimeMemoryError::Conflict {
                code: "worker_tool_already_active",
            }
        ));
        assert!(begin_worker_tool_error_invalidates_lease(
            &RuntimeMemoryError::LeaseLost {
                worker_run_id: Uuid::from_u128(7),
                attempt_epoch: 2,
            }
        ));
    }
}
