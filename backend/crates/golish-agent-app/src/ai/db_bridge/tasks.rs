//! Task / subtask domain methods for `GolishDbRepoProvider` (inherent `_impl`
//! layer). Bodies moved verbatim from the original `db_bridge.rs` trait impl;
//! the trait methods in `mod.rs` delegate here.

use uuid::Uuid;

use super::convert::*;
use super::GolishDbRepoProvider;
use golish_agent_kit::db_traits::*;

impl GolishDbRepoProvider {
    pub(super) async fn task_create_impl(&self, task: NewTask) -> anyhow::Result<TaskView> {
        let db_task = golish_db::repo::tasks::create(
            &self.pool,
            golish_db::models::NewTask {
                session_id: task.session_id,
                title: task.title,
                input: task.input,
            },
        )
        .await?;
        Ok(TaskView {
            id: db_task.id,
            input: db_task.input,
            status: convert_task_status(db_task.status),
            result: db_task.result,
        })
    }

    pub(super) async fn task_get_impl(&self, id: Uuid) -> anyhow::Result<Option<TaskView>> {
        let task = golish_db::repo::tasks::get(&self.pool, id).await?;
        Ok(task.map(|t| TaskView {
            id: t.id,
            input: t.input,
            status: convert_task_status(t.status),
            result: t.result,
        }))
    }

    pub(super) async fn task_update_status_impl(
        &self,
        id: Uuid,
        status: TaskStatus,
    ) -> anyhow::Result<()> {
        golish_db::repo::tasks::update_status(&self.pool, id, convert_task_status_back(status))
            .await?;
        Ok(())
    }

    pub(super) async fn task_set_result_impl(&self, id: Uuid, result: &str) -> anyhow::Result<()> {
        golish_db::repo::tasks::set_result(
            &self.pool,
            id,
            result,
            golish_db::models::TaskStatus::Finished,
        )
        .await?;
        Ok(())
    }

    pub(super) async fn subtask_create_impl(
        &self,
        task_id: Uuid,
        session_id: Uuid,
        title: &str,
        description: &str,
        agent: Option<AgentType>,
    ) -> anyhow::Result<SubtaskView> {
        let db_sub = golish_db::repo::subtasks::create(
            &self.pool,
            golish_db::repo::subtasks::NewSubtask {
                task_id,
                session_id,
                title: Some(title.to_string()),
                description: Some(description.to_string()),
                agent: agent.map(convert_agent_type_back),
            },
        )
        .await?;
        Ok(SubtaskView {
            id: db_sub.id,
            status: convert_subtask_status(db_sub.status),
            title: db_sub.title,
            description: db_sub.description,
            agent: db_sub.agent.map(convert_agent_type),
            result: db_sub.result,
        })
    }

    pub(super) async fn subtask_update_status_impl(
        &self,
        id: Uuid,
        status: SubtaskStatus,
    ) -> anyhow::Result<()> {
        golish_db::repo::subtasks::update_status(
            &self.pool,
            id,
            convert_subtask_status_back(status),
        )
        .await?;
        Ok(())
    }

    pub(super) async fn subtask_set_result_impl(
        &self,
        id: Uuid,
        result: &str,
    ) -> anyhow::Result<()> {
        golish_db::repo::subtasks::set_result(
            &self.pool,
            id,
            result,
            golish_db::models::SubtaskStatus::Finished,
        )
        .await?;
        Ok(())
    }

    pub(super) async fn subtask_next_pending_impl(
        &self,
        task_id: Uuid,
    ) -> anyhow::Result<Option<SubtaskView>> {
        let sub = golish_db::repo::subtasks::next_pending(&self.pool, task_id).await?;
        Ok(sub.map(|s| SubtaskView {
            id: s.id,
            status: convert_subtask_status(s.status),
            title: s.title,
            description: s.description,
            agent: s.agent.map(convert_agent_type),
            result: s.result,
        }))
    }

    pub(super) async fn subtask_list_by_task_impl(
        &self,
        task_id: Uuid,
    ) -> anyhow::Result<Vec<SubtaskView>> {
        let subs = golish_db::repo::subtasks::list_by_task(&self.pool, task_id).await?;
        Ok(subs
            .into_iter()
            .map(|s| SubtaskView {
                id: s.id,
                status: convert_subtask_status(s.status),
                title: s.title,
                description: s.description,
                agent: s.agent.map(convert_agent_type),
                result: s.result,
            })
            .collect())
    }

    pub(super) async fn subtask_delete_pending_impl(&self, task_id: Uuid) -> anyhow::Result<()> {
        golish_db::repo::subtasks::delete_pending(&self.pool, task_id).await?;
        Ok(())
    }
}
