//! PentAGI-style task-mode events: progress updates, subtask lifecycle,
//! interactive input, resume, and enricher results.

pub(super) fn task_progress(
    task_id: &str,
    status: &str,
    message: &str,
) -> serde_json::Value {
    serde_json::json!({
        "task_id": task_id,
        "status": status,
        "message": message
    })
}

pub(super) fn subtask_created(
    task_id: &str,
    subtask_id: &str,
    title: &str,
    agent: &Option<String>,
) -> serde_json::Value {
    serde_json::json!({
        "task_id": task_id,
        "subtask_id": subtask_id,
        "title": title,
        "agent": agent
    })
}

pub(super) fn subtask_completed(
    task_id: &str,
    subtask_id: &str,
    title: &str,
    result: &str,
) -> serde_json::Value {
    serde_json::json!({
        "task_id": task_id,
        "subtask_id": subtask_id,
        "title": title,
        "result": result
    })
}

pub(super) fn subtask_waiting_for_input(
    task_id: &str,
    subtask_id: &str,
    title: &str,
    prompt: &str,
) -> serde_json::Value {
    serde_json::json!({
        "task_id": task_id,
        "subtask_id": subtask_id,
        "title": title,
        "prompt": prompt
    })
}

pub(super) fn subtask_user_input(
    task_id: &str,
    subtask_id: &str,
    input: &str,
) -> serde_json::Value {
    serde_json::json!({
        "task_id": task_id,
        "subtask_id": subtask_id,
        "input": input
    })
}

pub(super) fn task_resumed(
    task_id: &str,
    subtask_index: usize,
    total_subtasks: usize,
) -> serde_json::Value {
    serde_json::json!({
        "task_id": task_id,
        "subtask_index": subtask_index,
        "total_subtasks": total_subtasks
    })
}

pub(super) fn enricher_result(
    task_id: &str,
    subtask_id: &str,
    context_added: &str,
) -> serde_json::Value {
    serde_json::json!({
        "task_id": task_id,
        "subtask_id": subtask_id,
        "context_added": context_added
    })
}
