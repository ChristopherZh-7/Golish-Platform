use golish_core::events::AiEvent;

use super::{hitl, task, CliJsonEvent};

pub(super) fn convert_hitl_or_task(event: &AiEvent) -> CliJsonEvent {
    match event {
        AiEvent::AskHumanRequest {
            request_id,
            question,
            input_type,
            options,
            context,
        } => CliJsonEvent::new(
            "ask_human_request",
            hitl::ask_human_request(request_id, question, input_type, options, context),
        ),
        AiEvent::AskHumanResponse {
            request_id,
            response,
            skipped,
        } => CliJsonEvent::new(
            "ask_human_response",
            hitl::ask_human_response(request_id, response, *skipped),
        ),
        AiEvent::TaskProgress {
            task_id,
            status,
            message,
        } => CliJsonEvent::new(
            "task_progress",
            task::task_progress(task_id, status, message),
        ),
        AiEvent::SubtaskCreated {
            task_id,
            subtask_id,
            title,
            agent,
        } => CliJsonEvent::new(
            "subtask_created",
            task::subtask_created(task_id, subtask_id, title, agent),
        ),
        AiEvent::SubtaskCompleted {
            task_id,
            subtask_id,
            title,
            result,
        } => CliJsonEvent::new(
            "subtask_completed",
            task::subtask_completed(task_id, subtask_id, title, result),
        ),
        AiEvent::SubtaskWaitingForInput {
            task_id,
            subtask_id,
            title,
            prompt,
        } => CliJsonEvent::new(
            "subtask_waiting_for_input",
            task::subtask_waiting_for_input(task_id, subtask_id, title, prompt),
        ),
        AiEvent::SubtaskUserInput {
            task_id,
            subtask_id,
            input,
        } => CliJsonEvent::new(
            "subtask_user_input",
            task::subtask_user_input(task_id, subtask_id, input),
        ),
        AiEvent::TaskResumed {
            task_id,
            subtask_index,
            total_subtasks,
        } => CliJsonEvent::new(
            "task_resumed",
            task::task_resumed(task_id, *subtask_index, *total_subtasks),
        ),
        AiEvent::EnricherResult {
            task_id,
            subtask_id,
            context_added,
        } => CliJsonEvent::new(
            "enricher_result",
            task::enricher_result(task_id, subtask_id, context_added),
        ),
        _ => unreachable!("caller only routes HITL and task events here"),
    }
}
