//! App-layer implementation of `DbRepoProvider` backed by `golish-db` repo
//! functions and a `PgPool`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use golish_ai::db_traits::*;

pub struct GolishDbRepoProvider {
    pool: Arc<PgPool>,
}

impl GolishDbRepoProvider {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DbRepoProvider for GolishDbRepoProvider {
    // -- Wiki KB --
    async fn wiki_upsert_page(&self, page: &NewWikiPage) -> anyhow::Result<()> {
        let db_page = golish_db::models::NewWikiPage {
            path: page.path.clone(),
            title: page.title.clone(),
            category: page.category.clone(),
            tags: page.tags.clone(),
            status: page.status.clone(),
            content: page.content.clone(),
        };
        golish_db::repo::wiki_kb::upsert_page(&self.pool, &db_page).await?;
        Ok(())
    }

    async fn wiki_link_cve(&self, cve: &str, path: &str) -> anyhow::Result<()> {
        golish_db::repo::wiki_kb::link_cve_to_wiki(&self.pool, cve, path).await?;
        Ok(())
    }

    async fn wiki_delete_refs_from(&self, path: &str) -> anyhow::Result<()> {
        golish_db::repo::wiki_kb::delete_refs_from(&self.pool, path).await?;
        Ok(())
    }

    async fn wiki_upsert_page_ref(&self, from: &str, to: &str, ctx: &str) -> anyhow::Result<()> {
        golish_db::repo::wiki_kb::upsert_page_ref(&self.pool, from, to, ctx).await?;
        Ok(())
    }

    async fn wiki_add_changelog(&self, entry: &NewWikiChangelog) -> anyhow::Result<()> {
        let db_entry = golish_db::models::NewWikiChangelog {
            page_path: entry.page_path.clone(),
            action: entry.action.clone(),
            title: entry.title.clone(),
            category: entry.category.clone(),
            actor: entry.actor.clone(),
            summary: entry.summary.clone(),
        };
        golish_db::repo::wiki_kb::add_changelog(&self.pool, &db_entry).await?;
        Ok(())
    }

    async fn wiki_search_fts(&self, query: &str, limit: i64) -> anyhow::Result<serde_json::Value> {
        let results = golish_db::repo::wiki_kb::search_fts(&self.pool, query, limit).await?;
        Ok(serde_json::to_value(results)?)
    }

    async fn wiki_search_by_category(&self, cat: &str, limit: i64) -> anyhow::Result<serde_json::Value> {
        let results = golish_db::repo::wiki_kb::search_by_category(&self.pool, cat, limit).await?;
        Ok(serde_json::to_value(results)?)
    }

    async fn wiki_search_by_tag(&self, tag: &str, limit: i64) -> anyhow::Result<serde_json::Value> {
        let results = golish_db::repo::wiki_kb::search_by_tag(&self.pool, tag, limit).await?;
        Ok(serde_json::to_value(results)?)
    }

    async fn wiki_list_cves_with_pocs(&self) -> anyhow::Result<serde_json::Value> {
        let rows = golish_db::repo::wiki_kb::list_cves_with_pocs(&self.pool).await?;
        Ok(serde_json::to_value(rows)?)
    }

    async fn wiki_list_unresearched_cves(&self, limit: i64) -> anyhow::Result<serde_json::Value> {
        let rows = golish_db::repo::wiki_kb::list_unresearched_cves(&self.pool, limit).await?;
        Ok(serde_json::to_value(rows)?)
    }

    async fn wiki_poc_stats(&self) -> anyhow::Result<serde_json::Value> {
        let stats = golish_db::repo::wiki_kb::poc_stats(&self.pool).await?;
        Ok(stats)
    }

    async fn wiki_upsert_poc_full(
        &self, cve_id: &str, name: &str, poc_type: &str, language: &str,
        content: &str, source: &str, source_url: &str, severity: &str,
        description: &str, tags: &[String],
    ) -> anyhow::Result<serde_json::Value> {
        let result = golish_db::repo::wiki_kb::upsert_poc_full(
            &self.pool, cve_id, name, poc_type, language, content,
            source, source_url, severity, description, tags,
        ).await?;
        Ok(serde_json::to_value(result)?)
    }

    // -- Vuln Intel --
    async fn vuln_intel_search(&self, cve_id: &str, limit: i64) -> anyhow::Result<serde_json::Value> {
        let entries = golish_db::repo::vuln_intel::search_entries(&self.pool, cve_id, limit).await?;
        Ok(serde_json::to_value(entries)?)
    }

    // -- Security Analysis --
    async fn audit_log_operation(
        &self, summary: &str, op_type: &str, description: &str,
        project_path: Option<&str>, source: &str, target_id: Option<Uuid>,
        session_id: Option<&str>, tool_name: Option<&str>, status: &str,
        detail: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let entry = golish_db::repo::audit::log_operation(
            &self.pool, summary, op_type, description, project_path,
            source, target_id, session_id, tool_name, status, detail,
        ).await?;
        Ok(serde_json::to_value(entry)?)
    }

    async fn api_endpoints_insert(
        &self, target_id: Uuid, project_path: Option<&str>, url: &str,
        method: &str, path: &str, params: &serde_json::Value,
        raw_data: &serde_json::Value, auth_type: Option<&str>,
        source: &str, risk_level: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let result = golish_db::repo::api_endpoints::insert(
            &self.pool, target_id, project_path.unwrap_or(""), url, method, path,
            params, raw_data, auth_type, source, risk_level,
        ).await?;
        Ok(serde_json::to_value(result)?)
    }

    async fn js_analysis_insert(
        &self, target_id: Uuid, project_path: &str, url: &str,
        filename: &str, analysis: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let result = golish_db::repo::js_analysis::insert(
            &self.pool, target_id, project_path, url, filename,
            None, None,
            &json!([]), &json!([]), &json!([]), &json!([]),
            &json!([]), false, "", &json!({}),
        ).await?;
        Ok(serde_json::to_value(result)?)
    }

    async fn js_analysis_update_file_path(&self, id: Uuid, file_path: &str) -> anyhow::Result<()> {
        golish_db::repo::js_analysis::update_file_path(&self.pool, id, file_path).await?;
        Ok(())
    }

    async fn fingerprints_upsert(
        &self, target_id: Uuid, project_path: &str, category: &str,
        name: &str, version: Option<&str>, confidence: f64,
        raw_data: Option<&serde_json::Value>,
    ) -> anyhow::Result<bool> {
        let result = golish_db::repo::fingerprints::upsert(
            &self.pool, target_id, project_path, category, name,
            version, confidence as f32, raw_data.unwrap_or(&json!({})),
            None, "",
        ).await?;
        Ok(result)
    }

    async fn passive_scans_insert(
        &self, target_id: Uuid, project_path: &str, scan_type: &str,
        tool_name: &str, findings: &serde_json::Value,
        raw_output: Option<&str>, severity: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let result = golish_db::repo::passive_scans::insert(
            &self.pool, target_id, project_path,
            scan_type, "", "", "", "", raw_output.unwrap_or(""),
            severity, tool_name, "ai", "", &json!({}),
        ).await?;
        Ok(serde_json::to_value(result)?)
    }

    async fn query_target_data(
        &self, target_id: Uuid, sections: &[String],
    ) -> anyhow::Result<serde_json::Value> {
        let include_all = sections.contains(&"all".to_string());
        let mut data = json!({});

        if include_all || sections.contains(&"assets".to_string()) {
            if let Ok(assets) = golish_db::repo::target_assets::list_by_target(&self.pool, target_id).await {
                data["assets"] = serde_json::to_value(&assets)?;
                data["assets_count"] = json!(assets.len());
            }
        }
        if include_all || sections.contains(&"endpoints".to_string()) {
            if let Ok(endpoints) = golish_db::repo::api_endpoints::list_by_target(&self.pool, target_id).await {
                data["endpoints"] = serde_json::to_value(&endpoints)?;
                data["endpoints_count"] = json!(endpoints.len());
            }
        }
        if include_all || sections.contains(&"fingerprints".to_string()) {
            if let Ok(fps) = golish_db::repo::fingerprints::list_by_target(&self.pool, target_id).await {
                data["fingerprints"] = serde_json::to_value(&fps)?;
            }
        }
        if include_all || sections.contains(&"js_analysis".to_string()) {
            if let Ok(results) = golish_db::repo::js_analysis::list_by_target(&self.pool, target_id).await {
                data["js_analysis"] = serde_json::to_value(&results)?;
            }
        }
        if include_all || sections.contains(&"scan_logs".to_string()) {
            if let Ok(logs) = golish_db::repo::passive_scans::list_by_target(&self.pool, target_id, 100).await {
                data["scan_logs"] = serde_json::to_value(&logs)?;
                if let Ok(stats) = golish_db::repo::passive_scans::stats_by_target(&self.pool, target_id).await {
                    data["scan_stats"] = stats;
                }
            }
        }
        Ok(data)
    }

    // -- Tasks --
    async fn task_create(&self, task: NewTask) -> anyhow::Result<TaskView> {
        let db_task = golish_db::repo::tasks::create(&self.pool, golish_db::models::NewTask {
            session_id: task.session_id,
            title: task.title,
            input: task.input,
        }).await?;
        Ok(TaskView { id: db_task.id, input: db_task.input, status: convert_task_status(db_task.status), result: db_task.result })
    }

    async fn task_get(&self, id: Uuid) -> anyhow::Result<Option<TaskView>> {
        let task = golish_db::repo::tasks::get(&self.pool, id).await?;
        Ok(task.map(|t| TaskView { id: t.id, input: t.input, status: convert_task_status(t.status), result: t.result }))
    }

    async fn task_update_status(&self, id: Uuid, status: TaskStatus) -> anyhow::Result<()> {
        golish_db::repo::tasks::update_status(&self.pool, id, convert_task_status_back(status)).await?;
        Ok(())
    }

    async fn task_set_result(&self, id: Uuid, result: &str) -> anyhow::Result<()> {
        golish_db::repo::tasks::set_result(&self.pool, id, result, golish_db::models::TaskStatus::Finished).await?;
        Ok(())
    }

    // -- Subtasks --
    async fn subtask_create(
        &self, task_id: Uuid, session_id: Uuid, title: &str,
        description: &str, agent: Option<AgentType>,
    ) -> anyhow::Result<SubtaskView> {
        let db_sub = golish_db::repo::subtasks::create(&self.pool, golish_db::repo::subtasks::NewSubtask {
            task_id,
            session_id,
            title: Some(title.to_string()),
            description: Some(description.to_string()),
            agent: agent.map(convert_agent_type_back),
        }).await?;
        Ok(SubtaskView {
            id: db_sub.id,
            status: convert_subtask_status(db_sub.status),
            title: db_sub.title,
            description: db_sub.description,
            agent: db_sub.agent.map(convert_agent_type),
            result: db_sub.result,
        })
    }

    async fn subtask_update_status(&self, id: Uuid, status: SubtaskStatus) -> anyhow::Result<()> {
        golish_db::repo::subtasks::update_status(&self.pool, id, convert_subtask_status_back(status)).await?;
        Ok(())
    }

    async fn subtask_set_result(&self, id: Uuid, result: &str) -> anyhow::Result<()> {
        golish_db::repo::subtasks::set_result(&self.pool, id, result, golish_db::models::SubtaskStatus::Finished).await?;
        Ok(())
    }

    async fn subtask_next_pending(&self, task_id: Uuid) -> anyhow::Result<Option<SubtaskView>> {
        let sub = golish_db::repo::subtasks::next_pending(&self.pool, task_id).await?;
        Ok(sub.map(|s| SubtaskView {
            id: s.id, status: convert_subtask_status(s.status),
            title: s.title, description: s.description,
            agent: s.agent.map(convert_agent_type), result: s.result,
        }))
    }

    async fn subtask_list_by_task(&self, task_id: Uuid) -> anyhow::Result<Vec<SubtaskView>> {
        let subs = golish_db::repo::subtasks::list_by_task(&self.pool, task_id).await?;
        Ok(subs.into_iter().map(|s| SubtaskView {
            id: s.id, status: convert_subtask_status(s.status),
            title: s.title, description: s.description,
            agent: s.agent.map(convert_agent_type), result: s.result,
        }).collect())
    }

    async fn subtask_delete_pending(&self, task_id: Uuid) -> anyhow::Result<()> {
        golish_db::repo::subtasks::delete_pending(&self.pool, task_id).await?;
        Ok(())
    }

    // -- Message Chains --
    async fn message_chain_create(
        &self, session_id: Uuid, task_id: Option<Uuid>, subtask_id: Option<Uuid>,
        agent_type: AgentType, parent_chain_id: Option<Uuid>, model: Option<&str>,
    ) -> anyhow::Result<MessageChainView> {
        let chain = golish_db::repo::message_chains::create(
            &self.pool, session_id, task_id, subtask_id,
            convert_agent_type_back(agent_type), parent_chain_id, model,
        ).await?;
        Ok(MessageChainView { id: chain.id })
    }

    async fn message_chain_update_chain(&self, id: Uuid, chain_json: &serde_json::Value) -> anyhow::Result<()> {
        golish_db::repo::message_chains::update_chain(&self.pool, id, chain_json).await?;
        Ok(())
    }

    async fn message_chain_update_usage(
        &self, id: Uuid, input_tokens: i32, output_tokens: i32,
        cache_read_tokens: i32, input_cost: f64, output_cost: f64, duration_ms: i32,
    ) -> anyhow::Result<()> {
        golish_db::repo::message_chains::update_usage(
            &self.pool, id, input_tokens, output_tokens,
            cache_read_tokens, input_cost, output_cost, duration_ms,
        ).await?;
        Ok(())
    }

    // -- Execution Plans --
    async fn plan_list_active(&self, project_path: &str) -> anyhow::Result<Vec<ExecutionPlanView>> {
        let plans = golish_db::repo::execution_plans::list_active(&self.pool, project_path).await?;
        Ok(plans.into_iter().map(|p| ExecutionPlanView {
            id: p.id, title: p.title, description: p.description,
            steps: p.steps, status: convert_plan_status(p.status), current_step: p.current_step,
        }).collect())
    }

    async fn plan_update_steps(
        &self, id: Uuid, steps: &serde_json::Value, current_step: i32, status: PlanStatus,
    ) -> anyhow::Result<()> {
        golish_db::repo::execution_plans::update_steps(
            &self.pool, id, steps, current_step, convert_plan_status_back(status),
        ).await?;
        Ok(())
    }

    async fn plan_create(&self, plan: NewExecutionPlan) -> anyhow::Result<ExecutionPlanView> {
        let created = golish_db::repo::execution_plans::create(
            &self.pool,
            golish_db::models::NewExecutionPlan {
                session_id: plan.session_id,
                project_path: plan.project_path,
                title: plan.title,
                description: plan.description,
                steps: plan.steps,
            },
        ).await?;
        Ok(ExecutionPlanView {
            id: created.id, title: created.title, description: created.description,
            steps: created.steps, status: convert_plan_status(created.status),
            current_step: created.current_step,
        })
    }
}

fn convert_task_status(s: golish_db::models::TaskStatus) -> TaskStatus {
    match s {
        golish_db::models::TaskStatus::Created => TaskStatus::Created,
        golish_db::models::TaskStatus::Running => TaskStatus::Running,
        golish_db::models::TaskStatus::Waiting => TaskStatus::Waiting,
        golish_db::models::TaskStatus::Finished => TaskStatus::Finished,
        golish_db::models::TaskStatus::Failed => TaskStatus::Failed,
    }
}

fn convert_task_status_back(s: TaskStatus) -> golish_db::models::TaskStatus {
    match s {
        TaskStatus::Created => golish_db::models::TaskStatus::Created,
        TaskStatus::Running => golish_db::models::TaskStatus::Running,
        TaskStatus::Waiting => golish_db::models::TaskStatus::Waiting,
        TaskStatus::Finished => golish_db::models::TaskStatus::Finished,
        TaskStatus::Failed => golish_db::models::TaskStatus::Failed,
    }
}

fn convert_subtask_status(s: golish_db::models::SubtaskStatus) -> SubtaskStatus {
    match s {
        golish_db::models::SubtaskStatus::Created => SubtaskStatus::Created,
        golish_db::models::SubtaskStatus::Running => SubtaskStatus::Running,
        golish_db::models::SubtaskStatus::Waiting => SubtaskStatus::Waiting,
        golish_db::models::SubtaskStatus::Finished => SubtaskStatus::Finished,
        golish_db::models::SubtaskStatus::Failed => SubtaskStatus::Failed,
    }
}

fn convert_subtask_status_back(s: SubtaskStatus) -> golish_db::models::SubtaskStatus {
    match s {
        SubtaskStatus::Created => golish_db::models::SubtaskStatus::Created,
        SubtaskStatus::Running => golish_db::models::SubtaskStatus::Running,
        SubtaskStatus::Waiting => golish_db::models::SubtaskStatus::Waiting,
        SubtaskStatus::Finished => golish_db::models::SubtaskStatus::Finished,
        SubtaskStatus::Failed => golish_db::models::SubtaskStatus::Failed,
    }
}

fn convert_agent_type(a: golish_db::models::AgentType) -> AgentType {
    match a {
        golish_db::models::AgentType::Primary => AgentType::Primary,
        golish_db::models::AgentType::Pentester => AgentType::Pentester,
        golish_db::models::AgentType::Coder => AgentType::Coder,
        golish_db::models::AgentType::Searcher => AgentType::Searcher,
        golish_db::models::AgentType::Memorist => AgentType::Memorist,
        golish_db::models::AgentType::Reporter => AgentType::Reporter,
        golish_db::models::AgentType::Adviser => AgentType::Adviser,
        golish_db::models::AgentType::Reflector => AgentType::Reflector,
        golish_db::models::AgentType::Enricher => AgentType::Enricher,
        golish_db::models::AgentType::Installer => AgentType::Installer,
    }
}

fn convert_agent_type_back(a: AgentType) -> golish_db::models::AgentType {
    match a {
        AgentType::Primary => golish_db::models::AgentType::Primary,
        AgentType::Pentester => golish_db::models::AgentType::Pentester,
        AgentType::Coder => golish_db::models::AgentType::Coder,
        AgentType::Searcher => golish_db::models::AgentType::Searcher,
        AgentType::Memorist => golish_db::models::AgentType::Memorist,
        AgentType::Reporter => golish_db::models::AgentType::Reporter,
        AgentType::Adviser => golish_db::models::AgentType::Adviser,
        AgentType::Reflector => golish_db::models::AgentType::Reflector,
        AgentType::Enricher => golish_db::models::AgentType::Enricher,
        AgentType::Installer => golish_db::models::AgentType::Installer,
    }
}

fn convert_plan_status(s: golish_db::models::PlanStatus) -> PlanStatus {
    match s {
        golish_db::models::PlanStatus::Planning => PlanStatus::Planning,
        golish_db::models::PlanStatus::InProgress => PlanStatus::InProgress,
        golish_db::models::PlanStatus::Paused => PlanStatus::Paused,
        golish_db::models::PlanStatus::Completed => PlanStatus::Completed,
        golish_db::models::PlanStatus::Failed => PlanStatus::Failed,
        golish_db::models::PlanStatus::Cancelled => PlanStatus::Cancelled,
    }
}

fn convert_plan_status_back(s: PlanStatus) -> golish_db::models::PlanStatus {
    match s {
        PlanStatus::Planning => golish_db::models::PlanStatus::Planning,
        PlanStatus::InProgress => golish_db::models::PlanStatus::InProgress,
        PlanStatus::Paused => golish_db::models::PlanStatus::Paused,
        PlanStatus::Completed => golish_db::models::PlanStatus::Completed,
        PlanStatus::Failed => golish_db::models::PlanStatus::Failed,
        PlanStatus::Cancelled => golish_db::models::PlanStatus::Cancelled,
    }
}
