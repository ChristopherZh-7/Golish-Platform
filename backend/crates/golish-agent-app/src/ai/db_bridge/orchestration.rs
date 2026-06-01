//! Message-chain / execution-plan / sub-agent-dispatch domain methods for
//! `GolishDbRepoProvider` (inherent `_impl` layer). Bodies moved verbatim from
//! the original `db_bridge.rs` trait impl; the trait methods in `mod.rs`
//! delegate here.

use uuid::Uuid;

use super::convert::*;
use super::GolishDbRepoProvider;
use golish_agent_kit::db_traits::*;

impl GolishDbRepoProvider {
    pub(super) async fn operation_state_insert_impl(
        &self,
        operation_id: Uuid,
        profile: &str,
        current_stage: &str,
    ) -> anyhow::Result<()> {
        golish_db::repo::operation_state::insert(&self.pool, operation_id, profile, current_stage)
            .await
            .map_err(Into::into)
    }

    pub(super) async fn operation_state_get_impl(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<Option<OperationStateView>> {
        let row = golish_db::repo::operation_state::get(&self.pool, operation_id).await?;
        Ok(row.map(|r| OperationStateView {
            operation_id: r.operation_id,
            profile: r.profile,
            current_stage: r.current_stage,
        }))
    }

    pub(super) async fn operation_state_advance_stage_impl(
        &self,
        operation_id: Uuid,
        new_stage: &str,
    ) -> anyhow::Result<()> {
        golish_db::repo::operation_state::advance_stage(&self.pool, operation_id, new_stage)
            .await
            .map_err(Into::into)
    }

    pub(super) async fn message_chain_create_impl(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        agent_type: AgentType,
        _parent_chain_id: Option<Uuid>,
        model: Option<&str>,
    ) -> anyhow::Result<MessageChainView> {
        let chain = golish_db::repo::message_chains::create(
            &self.pool,
            session_id,
            task_id,
            subtask_id,
            convert_agent_type_back(agent_type),
            None,
            model,
        )
        .await?;
        Ok(MessageChainView { id: chain.id })
    }

    pub(super) async fn message_chain_update_chain_impl(
        &self,
        id: Uuid,
        chain_json: &serde_json::Value,
    ) -> anyhow::Result<()> {
        golish_db::repo::message_chains::update_chain(&self.pool, id, chain_json).await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn message_chain_update_usage_impl(
        &self,
        id: Uuid,
        input_tokens: i32,
        output_tokens: i32,
        cache_read_tokens: i32,
        input_cost: f64,
        output_cost: f64,
        duration_ms: i32,
    ) -> anyhow::Result<()> {
        golish_db::repo::message_chains::update_usage(
            &self.pool,
            id,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            input_cost,
            output_cost,
            duration_ms,
        )
        .await?;
        Ok(())
    }

    pub(super) async fn plan_list_active_impl(
        &self,
        project_path: &str,
    ) -> anyhow::Result<Vec<ExecutionPlanView>> {
        let plans = self.pentest_plan.plan_list_active(project_path).await?;
        Ok(plans
            .into_iter()
            .map(|p| ExecutionPlanView {
                id: p.id,
                title: p.title,
                description: p.description,
                steps: p.steps,
                status: convert_plan_status(p.status),
                current_step: p.current_step,
            })
            .collect())
    }

    pub(super) async fn plan_update_steps_impl(
        &self,
        id: Uuid,
        steps: &serde_json::Value,
        current_step: i32,
        status: PlanStatus,
    ) -> anyhow::Result<()> {
        self.pentest_plan
            .plan_update_steps(id, steps, current_step, convert_plan_status_back(status))
            .await?;
        Ok(())
    }

    pub(super) async fn plan_create_impl(
        &self,
        plan: NewExecutionPlan,
    ) -> anyhow::Result<ExecutionPlanView> {
        let created = self
            .pentest_plan
            .plan_create(golish_db::models::NewExecutionPlan {
                session_id: plan.session_id,
                project_path: plan.project_path,
                title: plan.title,
                description: plan.description,
                steps: plan.steps,
            })
            .await?;
        Ok(ExecutionPlanView {
            id: created.id,
            title: created.title,
            description: created.description,
            steps: created.steps,
            status: convert_plan_status(created.status),
            current_step: created.current_step,
        })
    }

    pub(super) async fn dispatch_record_start_impl(
        &self,
        session_id: Uuid,
        parent_dispatch_id: Option<Uuid>,
        agent_id: &str,
        tool_call_id: Option<&str>,
        depth: i32,
        args: &serde_json::Value,
    ) -> anyhow::Result<Uuid> {
        golish_db::repo::sub_agent_dispatches::record_start(
            &self.pool,
            session_id,
            parent_dispatch_id,
            agent_id,
            tool_call_id,
            depth,
            args,
        )
        .await
        .map_err(Into::into)
    }

    pub(super) async fn dispatch_record_finish_impl(
        &self,
        id: Uuid,
        status: DispatchStatus,
        result: Option<&serde_json::Value>,
        error_message: Option<&str>,
    ) -> anyhow::Result<()> {
        golish_db::repo::sub_agent_dispatches::record_finish(
            &self.pool,
            id,
            convert_dispatch_status_back(status),
            result,
            error_message,
        )
        .await
        .map_err(Into::into)
    }

    pub(super) async fn dispatch_list_running_impl(
        &self,
        session_id: Uuid,
    ) -> anyhow::Result<Vec<SubAgentDispatchView>> {
        let rows =
            golish_db::repo::sub_agent_dispatches::list_running(&self.pool, session_id).await?;
        Ok(rows
            .into_iter()
            .map(|r| SubAgentDispatchView {
                id: r.id,
                parent_dispatch_id: r.parent_dispatch_id,
                agent_id: r.agent_id,
                tool_call_id: r.tool_call_id,
                depth: r.depth,
                args: r.args,
                started_at: r.started_at,
            })
            .collect())
    }
}
