//! Build the effective system prompt for a sub-agent invocation.
//!
//! Composition order (matches the legacy monolithic loop):
//! 1. Optionally generate an optimized prompt via a non-streaming completion
//!    when [`SubAgentDefinition::prompt_template`] is set (worker pattern).
//! 2. Append the orchestrator briefing (if any).
//! 3. Inject any skills matched against the task description.
//! 4. Append the barrier-tool completion-requirement instruction.

use rig::completion::{AssistantContent, CompletionModel as RigCompletionModel, Message};
use rig::message::{Text, UserContent};
use rig::one_or_many::OneOrMany;

use crate::definition::SubAgentDefinition;
use crate::executor_types::{SubAgentExecutorContext, BARRIER_TOOL_NAME};
use golish_core::events::AiEvent;

/// Compose the final system prompt the sub-agent will run with.
pub(super) async fn assemble_effective_system_prompt<M>(
    agent_def: &SubAgentDefinition,
    task: &str,
    additional_context: &str,
    ctx: &SubAgentExecutorContext<'_>,
    parent_request_id: &str,
    model: &M,
) -> String
where
    M: RigCompletionModel + Sync,
{
    let agent_id = &agent_def.id;

    let mut effective = if let Some(ref template) = agent_def.prompt_template {
        generate_optimized_prompt(
            agent_id,
            template,
            task,
            additional_context,
            ctx,
            parent_request_id,
            model,
            &agent_def.system_prompt,
        )
        .await
    } else {
        agent_def.system_prompt.clone()
    };

    if let Some(ref briefing) = ctx.briefing {
        effective.push_str("\n\n");
        effective.push_str(briefing);
        tracing::info!(
            "[sub-agent:{}] Injected orchestrator briefing ({} chars)",
            agent_id,
            briefing.len()
        );
    }

    inject_matched_skills(agent_id, task, ctx, &mut effective).await;

    effective.push_str(&format!(
        "\n\n## COMPLETION REQUIREMENT\n\n\
         When your task is complete, you MUST call the `{}` tool to submit your result. \
         Do NOT end with a plain text message — always use `{}` with:\n\
         - `result`: your full findings, outputs, or deliverables\n\
         - `success`: true if the task was completed, false if it failed\n\
         - `summary`: a one-line summary of what was accomplished",
        BARRIER_TOOL_NAME, BARRIER_TOOL_NAME
    ));

    effective
}

/// Run the prompt-architect LLM call to translate a task description into a
/// fine-tuned worker system prompt. Falls back to the static
/// [`SubAgentDefinition::system_prompt`] on failure or empty response.
#[allow(clippy::too_many_arguments)]
async fn generate_optimized_prompt<M>(
    agent_id: &str,
    template: &str,
    task: &str,
    additional_context: &str,
    ctx: &SubAgentExecutorContext<'_>,
    parent_request_id: &str,
    model: &M,
    fallback_system_prompt: &str,
) -> String
where
    M: RigCompletionModel + Sync,
{
    let generation_input = if additional_context.is_empty() {
        format!("Task: {}", task)
    } else {
        format!("Task: {}\n\nContext: {}", task, additional_context)
    };

    tracing::info!(
        "[sub-agent:{}] Generating optimized system prompt via LLM call",
        agent_id
    );

    let _ = ctx.event_tx.send(AiEvent::PromptGenerationStarted {
        agent_id: agent_id.to_string(),
        parent_request_id: parent_request_id.to_string(),
        architect_system_prompt: template.to_string(),
        architect_user_message: generation_input.clone(),
    });

    let generation_start = std::time::Instant::now();

    let generation_request = rig::completion::CompletionRequest {
        preamble: Some(template.to_string()),
        chat_history: OneOrMany::one(Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: generation_input,
            })),
        }),
        documents: vec![],
        tools: vec![],
        temperature: Some(0.3),
        max_tokens: Some(2048),
        tool_choice: None,
        additional_params: None,
        model: None,
        output_schema: None,
    };

    match model.completion(generation_request).await {
        Ok(response) => {
            let generated = response
                .choice
                .iter()
                .filter_map(|c| {
                    if let AssistantContent::Text(t) = c {
                        Some(t.text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");

            if generated.trim().is_empty() {
                tracing::warn!(
                    "[sub-agent:{}] Prompt generation returned empty response, using default",
                    agent_id
                );
                let _ = ctx.event_tx.send(AiEvent::PromptGenerationCompleted {
                    agent_id: agent_id.to_string(),
                    parent_request_id: parent_request_id.to_string(),
                    generated_prompt: None,
                    success: false,
                    duration_ms: generation_start.elapsed().as_millis() as u64,
                });
                fallback_system_prompt.to_string()
            } else {
                tracing::info!(
                    "[sub-agent:{}] Generated system prompt ({} chars)",
                    agent_id,
                    generated.len()
                );
                let _ = ctx.event_tx.send(AiEvent::PromptGenerationCompleted {
                    agent_id: agent_id.to_string(),
                    parent_request_id: parent_request_id.to_string(),
                    generated_prompt: Some(generated.clone()),
                    success: true,
                    duration_ms: generation_start.elapsed().as_millis() as u64,
                });
                generated
            }
        }
        Err(e) => {
            tracing::warn!(
                "[sub-agent:{}] Prompt generation failed: {}. Using default system prompt.",
                agent_id,
                e
            );
            let _ = ctx.event_tx.send(AiEvent::PromptGenerationCompleted {
                agent_id: agent_id.to_string(),
                parent_request_id: parent_request_id.to_string(),
                generated_prompt: None,
                success: false,
                duration_ms: generation_start.elapsed().as_millis() as u64,
            });
            fallback_system_prompt.to_string()
        }
    }
}

/// Discover skills under the active workspace, match them against the task
/// description, and append matched skills as `<skill name="...">…</skill>`
/// blocks to the system prompt.
async fn inject_matched_skills(
    agent_id: &str,
    task: &str,
    ctx: &SubAgentExecutorContext<'_>,
    effective_system_prompt: &mut String,
) {
    let workspace = ctx.workspace.read().await;
    let workspace_str = workspace.to_string_lossy();
    let skills = golish_skills::discover_skills(Some(&workspace_str));
    if skills.is_empty() {
        return;
    }

    let metadata: Vec<golish_skills::SkillMetadata> =
        skills.iter().map(golish_skills::SkillMetadata::from).collect();
    let matcher = golish_skills::SkillMatcher::default();
    let matches = matcher.match_skills(task, &metadata);
    for (matched_meta, _score, reason) in &matches {
        if let Ok(body) = golish_skills::load_skill_body(&matched_meta.path) {
            effective_system_prompt.push_str("\n\n<skill name=\"");
            effective_system_prompt.push_str(&matched_meta.name);
            effective_system_prompt.push_str("\">\n");
            effective_system_prompt.push_str(&body);
            effective_system_prompt.push_str("\n</skill>");
            tracing::info!(
                "[sub-agent:{}] Injected skill '{}' ({} chars, reason: {})",
                agent_id,
                matched_meta.name,
                body.len(),
                reason
            );
        }
    }
}
