//! Message-chain / execution-plan / sub-agent-dispatch domain methods for
//! `GolishDbRepoProvider` (inherent `_impl` layer). Bodies moved verbatim from
//! the original `db_bridge.rs` trait impl; the trait methods in `mod.rs`
//! delegate here.

use uuid::Uuid;

use super::convert::*;
use super::GolishDbRepoProvider;
use golish_agent_kit::db_traits::*;
use golish_agent_kit::runtime_memory::RuntimeMemoryContract;

pub(super) fn stage_asset_wave_to_view(
    wave: golish_db::repo::stage_asset_waves::StageAssetWaveWithItems,
) -> StageAssetWaveView {
    let target_ids = wave.items.iter().map(|item| item.target_id).collect();
    StageAssetWaveView {
        id: wave.wave.id,
        operation_id: wave.wave.operation_id,
        organization_id: wave.wave.organization_id,
        stage_kind: wave.wave.stage_kind,
        wave_index: wave.wave.wave_index,
        started_at: wave.wave.started_at,
        parent_wave_id: wave.wave.parent_wave_id,
        asset_hash: wave.wave.asset_hash,
        target_ids,
        asset_values: wave
            .items
            .into_iter()
            .map(|item| item.asset_value)
            .collect(),
    }
}

impl GolishDbRepoProvider {
    pub(super) async fn operation_state_insert_impl(
        &self,
        operation_id: Uuid,
        profile: &str,
        current_stage: &str,
        runtime_memory_contract: RuntimeMemoryContract,
    ) -> anyhow::Result<()> {
        golish_db::repo::operation_state::insert(
            &self.pool,
            operation_id,
            profile,
            current_stage,
            runtime_memory_contract.as_str(),
        )
        .await
        .map_err(Into::into)
    }

    pub(super) async fn operation_state_get_impl(
        &self,
        operation_id: Uuid,
    ) -> anyhow::Result<Option<OperationStateView>> {
        let row = golish_db::repo::operation_state::get(&self.pool, operation_id).await?;
        row.map(|r| {
            let runtime_memory_contract =
                RuntimeMemoryContract::try_from(r.runtime_memory_contract.as_str())?;
            Ok(OperationStateView {
                operation_id: r.operation_id,
                profile: r.profile,
                current_stage: r.current_stage,
                runtime_memory_contract,
                project_scope_id: r.project_scope_id,
                engagement_org_id: r.engagement_org_id,
                state_blob: r.state_blob,
                stage_started_at: r.stage_started_at,
            })
        })
        .transpose()
    }

    pub(super) async fn operation_state_set_engagement_org_impl(
        &self,
        operation_id: Uuid,
        org_id: Option<Uuid>,
    ) -> anyhow::Result<()> {
        golish_db::repo::operation_state::set_engagement_org(&self.pool, operation_id, org_id)
            .await?;
        Ok(())
    }

    pub(super) async fn stage_run_insert_impl(
        &self,
        id: Uuid,
        operation_id: Uuid,
        stage_kind: &str,
    ) -> anyhow::Result<()> {
        golish_db::repo::stage_runs::insert(&self.pool, id, operation_id, stage_kind)
            .await
            .map_err(Into::into)
    }

    pub(super) async fn stage_run_mark_terminal_impl(
        &self,
        id: Uuid,
        status: &str,
    ) -> anyhow::Result<()> {
        golish_db::repo::stage_runs::mark_terminal(&self.pool, id, status)
            .await
            .map_err(Into::into)
    }

    pub(super) async fn stage_asset_wave_current_or_create_initial_impl(
        &self,
        stage_execution_id: Uuid,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
        started_at: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        let wave = golish_db::repo::stage_asset_waves::current_or_create_initial(
            &self.pool,
            operation_id,
            organization_id,
            stage_kind,
            started_at,
            limit,
        )
        .await?;
        if let Some(wave) = &wave {
            self.seal_wave_before_dispatch(stage_execution_id, operation_id, wave.wave.id)
                .await?;
        }
        Ok(wave.map(stage_asset_wave_to_view))
    }

    pub(super) async fn stage_asset_wave_current_running_impl(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        golish_db::repo::stage_asset_waves::current_running(
            &self.pool,
            operation_id,
            organization_id,
            stage_kind,
        )
        .await
        .map(|maybe| maybe.map(stage_asset_wave_to_view))
        .map_err(Into::into)
    }

    pub(super) async fn stage_asset_wave_current_running_for_dispatch_impl(
        &self,
        stage_execution_id: Uuid,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        let wave = golish_db::repo::stage_asset_waves::current_running(
            &self.pool,
            operation_id,
            organization_id,
            stage_kind,
        )
        .await?;
        if let Some(wave) = &wave {
            self.seal_wave_before_dispatch(stage_execution_id, operation_id, wave.wave.id)
                .await?;
        }
        Ok(wave.map(stage_asset_wave_to_view))
    }

    pub(super) async fn stage_asset_wave_all_items_created_at_or_before_impl(
        &self,
        wave_id: Uuid,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<bool> {
        golish_db::repo::stage_asset_waves::all_items_created_at_or_before(
            &self.pool, wave_id, cutoff,
        )
        .await
        .map_err(Into::into)
    }

    pub(super) async fn stage_asset_wave_create_next_impl(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
        parent_wave_id: Option<Uuid>,
        limit: i64,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        golish_db::repo::stage_asset_waves::create_next(
            &self.pool,
            operation_id,
            organization_id,
            stage_kind,
            parent_wave_id,
            limit,
        )
        .await
        .map(|maybe| maybe.map(stage_asset_wave_to_view))
        .map_err(Into::into)
    }

    pub(super) async fn stage_asset_wave_create_next_or_seal_completion_impl(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_kind: &str,
        parent_wave_id: Option<Uuid>,
        limit: i64,
        stage_run_id: Option<&str>,
    ) -> anyhow::Result<Option<StageAssetWaveView>> {
        golish_db::repo::stage_asset_waves::create_next_or_seal_completion(
            &self.pool,
            operation_id,
            organization_id,
            stage_kind,
            parent_wave_id,
            limit,
            stage_run_id,
        )
        .await
        .map(|maybe| maybe.map(stage_asset_wave_to_view))
        .map_err(Into::into)
    }

    pub(super) async fn stage_asset_wave_complete_impl(&self, wave_id: Uuid) -> anyhow::Result<()> {
        golish_db::repo::stage_asset_waves::complete(&self.pool, wave_id)
            .await
            .map_err(Into::into)
    }

    pub(super) async fn operation_state_write_state_blob_impl(
        &self,
        operation_id: Uuid,
        state_blob: serde_json::Value,
    ) -> anyhow::Result<()> {
        golish_db::repo::operation_state::write_state_blob(&self.pool, operation_id, state_blob)
            .await
            .map_err(Into::into)
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
                stage_id: p.stage_id,
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
                stage_id: plan.stage_id,
            })
            .await?;
        Ok(ExecutionPlanView {
            id: created.id,
            title: created.title,
            description: created.description,
            steps: created.steps,
            status: convert_plan_status(created.status),
            current_step: created.current_step,
            stage_id: created.stage_id,
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

#[cfg(test)]
mod tests {
    #[test]
    fn stage_asset_wave_seals_denominator_before_return() {
        let source = include_str!("orchestration.rs");
        for function in [
            "stage_asset_wave_current_or_create_initial_impl",
            "stage_asset_wave_current_running_for_dispatch_impl",
        ] {
            let start = source.find(function).expect("dispatch wave bridge exists");
            let body = &source[start..];
            let seal = body
                .find("seal_wave_before_dispatch")
                .expect("dispatch view is sealed");
            let returned = body
                .find("Ok(wave.map(stage_asset_wave_to_view))")
                .expect("dispatch view is returned");
            assert!(seal < returned, "{function} must seal before return");
        }
    }
}
