//! Thin shim that mirrors the `golish_db::repo::*` function signatures but
//! delegates to the [`DbRepoProvider`] trait. This minimizes caller changes —
//! files only need to update their imports and swap `pool` for `repo`.

use crate::db_traits::*;
use uuid::Uuid;

pub mod tasks {
    use super::*;

    pub async fn create(repo: &dyn DbRepoProvider, task: NewTask) -> anyhow::Result<TaskView> {
        repo.task_create(task).await
    }

    pub async fn update_status(
        repo: &dyn DbRepoProvider,
        id: Uuid,
        status: TaskStatus,
    ) -> anyhow::Result<()> {
        repo.task_update_status(id, status).await
    }

    pub async fn set_result(
        repo: &dyn DbRepoProvider,
        id: Uuid,
        result: &str,
        status: TaskStatus,
    ) -> anyhow::Result<()> {
        repo.task_set_result(id, result).await?;
        repo.task_update_status(id, status).await
    }

    pub async fn get(repo: &dyn DbRepoProvider, id: Uuid) -> anyhow::Result<Option<TaskView>> {
        repo.task_get(id).await
    }
}

pub mod subtasks {
    use super::*;

    pub struct NewSubtask {
        pub task_id: Uuid,
        pub session_id: Uuid,
        pub title: Option<String>,
        pub description: Option<String>,
        pub agent: Option<AgentType>,
    }

    pub async fn create(
        repo: &dyn DbRepoProvider,
        sub: NewSubtask,
    ) -> anyhow::Result<SubtaskView> {
        repo.subtask_create(
            sub.task_id,
            sub.session_id,
            sub.title.as_deref().unwrap_or(""),
            sub.description.as_deref().unwrap_or(""),
            sub.agent,
        )
        .await
    }

    pub async fn update_status(
        repo: &dyn DbRepoProvider,
        id: Uuid,
        status: SubtaskStatus,
    ) -> anyhow::Result<()> {
        repo.subtask_update_status(id, status).await
    }

    pub async fn set_result(
        repo: &dyn DbRepoProvider,
        id: Uuid,
        result: &str,
        status: SubtaskStatus,
    ) -> anyhow::Result<()> {
        repo.subtask_set_result(id, result).await?;
        repo.subtask_update_status(id, status).await
    }

    pub async fn next_pending(
        repo: &dyn DbRepoProvider,
        task_id: Uuid,
    ) -> anyhow::Result<Option<SubtaskView>> {
        repo.subtask_next_pending(task_id).await
    }

    pub async fn list_by_task(
        repo: &dyn DbRepoProvider,
        task_id: Uuid,
    ) -> anyhow::Result<Vec<SubtaskView>> {
        repo.subtask_list_by_task(task_id).await
    }

    pub async fn delete_pending(
        repo: &dyn DbRepoProvider,
        task_id: Uuid,
    ) -> anyhow::Result<()> {
        repo.subtask_delete_pending(task_id).await
    }
}

pub mod message_chains {
    use super::*;

    pub async fn create(
        repo: &dyn DbRepoProvider,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        agent_type: AgentType,
        parent_chain_id: Option<Uuid>,
        model: Option<&str>,
    ) -> anyhow::Result<MessageChainView> {
        repo.message_chain_create(
            session_id,
            task_id,
            subtask_id,
            agent_type,
            parent_chain_id,
            model,
        )
        .await
    }

    pub async fn update_chain(
        repo: &dyn DbRepoProvider,
        id: Uuid,
        chain_json: &serde_json::Value,
    ) -> anyhow::Result<()> {
        repo.message_chain_update_chain(id, chain_json).await
    }

    pub async fn update_usage(
        repo: &dyn DbRepoProvider,
        id: Uuid,
        input_tokens: i32,
        output_tokens: i32,
        cache_read_tokens: i32,
        input_cost: f64,
        output_cost: f64,
        duration_ms: i32,
    ) -> anyhow::Result<()> {
        repo.message_chain_update_usage(
            id,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            input_cost,
            output_cost,
            duration_ms,
        )
        .await
    }
}

pub mod execution_plans {
    use super::*;

    pub async fn list_active(
        repo: &dyn DbRepoProvider,
        project_path: &str,
    ) -> anyhow::Result<Vec<ExecutionPlanView>> {
        repo.plan_list_active(project_path).await
    }

    pub async fn update_steps(
        repo: &dyn DbRepoProvider,
        id: Uuid,
        steps: &serde_json::Value,
        current_step: i32,
        status: PlanStatus,
    ) -> anyhow::Result<()> {
        repo.plan_update_steps(id, steps, current_step, status)
            .await
    }

    pub async fn create(
        repo: &dyn DbRepoProvider,
        plan: NewExecutionPlan,
    ) -> anyhow::Result<ExecutionPlanView> {
        repo.plan_create(plan).await
    }
}

pub mod wiki_kb {
    use super::*;

    pub async fn upsert_page(
        repo: &dyn DbRepoProvider,
        page: &NewWikiPage,
    ) -> anyhow::Result<()> {
        repo.wiki_upsert_page(page).await
    }

    pub async fn link_cve_to_wiki(
        repo: &dyn DbRepoProvider,
        cve: &str,
        path: &str,
    ) -> anyhow::Result<()> {
        repo.wiki_link_cve(cve, path).await
    }

    pub async fn delete_refs_from(
        repo: &dyn DbRepoProvider,
        path: &str,
    ) -> anyhow::Result<()> {
        repo.wiki_delete_refs_from(path).await
    }

    pub async fn upsert_page_ref(
        repo: &dyn DbRepoProvider,
        from_path: &str,
        to_path: &str,
        context: &str,
    ) -> anyhow::Result<()> {
        repo.wiki_upsert_page_ref(from_path, to_path, context).await
    }

    pub async fn add_changelog(
        repo: &dyn DbRepoProvider,
        entry: &NewWikiChangelog,
    ) -> anyhow::Result<()> {
        repo.wiki_add_changelog(entry).await
    }

    pub async fn search_fts(
        repo: &dyn DbRepoProvider,
        query: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        repo.wiki_search_fts(query, limit).await
    }

    pub async fn search_by_category(
        repo: &dyn DbRepoProvider,
        category: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        repo.wiki_search_by_category(category, limit).await
    }

    pub async fn search_by_tag(
        repo: &dyn DbRepoProvider,
        tag: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        repo.wiki_search_by_tag(tag, limit).await
    }

    pub async fn list_cves_with_pocs(
        repo: &dyn DbRepoProvider,
    ) -> anyhow::Result<serde_json::Value> {
        repo.wiki_list_cves_with_pocs().await
    }

    pub async fn list_unresearched_cves(
        repo: &dyn DbRepoProvider,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        repo.wiki_list_unresearched_cves(limit).await
    }

    pub async fn poc_stats(
        repo: &dyn DbRepoProvider,
    ) -> anyhow::Result<serde_json::Value> {
        repo.wiki_poc_stats().await
    }

    pub async fn upsert_poc_full(
        repo: &dyn DbRepoProvider,
        cve_id: &str,
        name: &str,
        poc_type: &str,
        language: &str,
        content: &str,
        source: &str,
        source_url: &str,
        severity: &str,
        description: &str,
        tags: &[String],
    ) -> anyhow::Result<serde_json::Value> {
        repo.wiki_upsert_poc_full(
            cve_id, name, poc_type, language, content, source, source_url,
            severity, description, tags,
        )
        .await
    }
}

pub mod vuln_intel {
    use super::*;

    pub async fn search_entries(
        repo: &dyn DbRepoProvider,
        cve_id: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        repo.vuln_intel_search(cve_id, limit).await
    }
}

pub mod audit {
    use super::*;

    pub async fn log_operation(
        repo: &dyn DbRepoProvider,
        summary: &str,
        op_type: &str,
        description: &str,
        project_path: Option<&str>,
        source: &str,
        target_id: Option<Uuid>,
        session_id: Option<&str>,
        tool_name: Option<&str>,
        status: &str,
        detail: &serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        repo.audit_log_operation(
            summary,
            op_type,
            description,
            project_path,
            source,
            target_id,
            session_id,
            tool_name,
            status,
            detail,
        )
        .await
    }
}
