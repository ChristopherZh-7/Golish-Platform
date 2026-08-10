//! Pipeline phase prompts: reflector, mentor, enricher, planner, and task wrapping.

use super::safe_truncate;

/// Reflector system prompt — guides LLM back to tool usage when it returns plain text.
///
/// Mirrors PentAGI's `reflector.tmpl`: acts as a proxy user who redirects
/// the agent to use structured tool calls instead of freeform text.
pub fn reflector_system_prompt() -> &'static str {
    r#"You are a task execution coordinator reviewing an AI agent's work.

## YOUR ROLE

The agent was given a specific subtask but responded with plain text instead of
executing actions using its available tools. Your job is to redirect the agent
back to productive tool usage.

## COMMUNICATION STYLE

- Be direct and concise — no greetings or pleasantries
- Respond as if you're the user who requested the task
- Keep your response under 200 words
- Focus on actionable next steps

## INSTRUCTIONS

1. Acknowledge what the agent said (briefly)
2. Explain that talking about the task is not the same as doing it
3. Suggest specific tools or actions the agent should take
4. Remind the agent that it must USE TOOLS to make progress, not just describe what it would do

If the agent asked a question, answer it directly, then redirect to tool usage.
If the agent is confused, clarify the objective and suggest the first concrete action.
"#
}

/// Execution Mentor system prompt — monitors agent progress and provides corrective advice.
///
/// Mirrors PentAGI's `performMentor()` pattern: when the execution monitor
/// detects repetitive tool usage, the mentor analyzes the situation and
/// provides strategic advice that is injected into the tool response.
pub fn mentor_system_prompt() -> &'static str {
    r#"You are an execution monitor for an AI agent performing a penetration testing task.

## YOUR ROLE

The agent appears to be making suboptimal tool choices — calling the same tools
repeatedly or not making meaningful progress. Review the execution history and
provide strategic guidance.

## STAGE BOUNDARY

If the user prompt includes a current stage, agent role, allowed tools, or allowed
scan categories, that boundary is mandatory. Do not recommend tools that are not
visible to the agent. Do not recommend exploitation, credential attacks,
vulnerability scanning, brute forcing, or next-stage actions unless the current
stage explicitly permits that kind of work.

## INSTRUCTIONS

1. Analyze what the agent has done so far
2. Identify why it might be stuck (wrong approach, missing context, repeated errors)
3. Suggest a specific alternative strategy or next tool to use
4. Keep the advice inside the current stage's gate objective
5. Be concise (under 150 words) and actionable

## OUTPUT

Provide advice as a direct message to the agent. No headers or formatting — just
clear, actionable guidance on what to do differently.
"#
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MentorPromptContext {
    pub stage: Option<String>,
    pub agent_role: Option<String>,
    pub allowed_tools: Vec<String>,
    pub allowed_tool_types: Vec<String>,
}

impl MentorPromptContext {
    fn has_any_boundary(&self) -> bool {
        self.stage.as_deref().is_some_and(|s| !s.trim().is_empty())
            || self
                .agent_role
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            || !self.allowed_tools.is_empty()
            || !self.allowed_tool_types.is_empty()
    }
}

fn join_limited(items: &[String], limit: usize) -> String {
    if items.is_empty() {
        return "(not provided)".to_string();
    }
    let mut rendered = items
        .iter()
        .take(limit)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > limit {
        rendered.push_str(&format!(", +{} more", items.len() - limit));
    }
    rendered
}

fn mentor_stage_objective(stage: Option<&str>) -> &'static str {
    match stage.unwrap_or_default() {
        "target_intel" => {
            "Advance the adaptive corporate-intelligence Goal from current durable facts: choose high-information semantic pivots, resolve material frontier items, preserve evidence and attribution, then request neutral review. Do not run active scans."
        }
        "external_attack_surface" => {
            "Close EAS coverage: liveness, port discovery, and service fingerprint cells must reach terminal states. Wait for background jobs, inspect outputs, update structured evidence, then submit."
        }
        "enumeration" => {
            "Close authorized enumeration coverage and consolidate discovered endpoints. Do not jump to exploitation."
        }
        "vuln_triage" | "verification" | "exploitation" => {
            "Stay within the current vulnerability-validation gate and cite real evidence."
        }
        _ => "Stay within the current stage gate and cite real evidence.",
    }
}

fn render_mentor_context(context: &MentorPromptContext) -> String {
    if !context.has_any_boundary() {
        return String::new();
    }
    let stage = context.stage.as_deref().unwrap_or("(not provided)");
    let role = context.agent_role.as_deref().unwrap_or("(not provided)");
    format!(
        r#"
Current execution boundary:
- Current stage: {stage}
- Agent role: {role}
- Allowed visible tools: {allowed_tools}
- Allowed scan categories: {allowed_tool_types}
- Stage objective: {objective}

Boundary rules:
- Stay inside the current stage and its gate objective.
- Use only tools that are visible/allowed for this agent.
- Do NOT recommend exploitation, credential attacks, brute force, vulnerability scanning, or next-stage work unless the current stage explicitly permits it.
- If background jobs are pending or recent scan output has not been reviewed, prefer wait/read/consolidate/submit over launching more scans.
"#,
        stage = stage,
        role = role,
        allowed_tools = safe_truncate(&join_limited(&context.allowed_tools, 32), 1200),
        allowed_tool_types = safe_truncate(&join_limited(&context.allowed_tool_types, 24), 800),
        objective = mentor_stage_objective(context.stage.as_deref()),
    )
}

pub fn mentor_hard_stage_guard(context: &MentorPromptContext) -> String {
    if !context.has_any_boundary() {
        return "Current stage boundary: obey the active harness stage, use only visible tools, and never bypass gate/submit requirements.".to_string();
    }
    let stage = context.stage.as_deref().unwrap_or("(not provided)");
    let role = context.agent_role.as_deref().unwrap_or("(not provided)");
    format!(
        "Current stage boundary:\n\
         - Current stage: {stage}\n\
         - Agent role: {role}\n\
         - Allowed visible tools: {allowed_tools}\n\
         - Allowed scan categories: {allowed_tool_types}\n\
         - Objective now: {objective}\n\
         - Do NOT recommend or follow exploitation, credential attacks, brute force, vulnerability scanning, or next-stage work unless this exact stage permits it.\n\
         - If the model advice below conflicts with this boundary, obey this boundary.",
        stage = stage,
        role = role,
        allowed_tools = safe_truncate(&join_limited(&context.allowed_tools, 32), 1200),
        allowed_tool_types = safe_truncate(&join_limited(&context.allowed_tool_types, 24), 800),
        objective = mentor_stage_objective(context.stage.as_deref()),
    )
}

/// Execution Mentor user prompt — provides context about the stuck agent.
pub fn mentor_user_prompt(
    subtask_description: &str,
    repeated_tool: &str,
    repeat_count: usize,
    recent_tool_calls: &str,
) -> String {
    mentor_user_prompt_with_context(
        subtask_description,
        repeated_tool,
        repeat_count,
        recent_tool_calls,
        &MentorPromptContext::default(),
    )
}

/// Execution Mentor user prompt with stage/role/tool boundary context.
pub fn mentor_user_prompt_with_context(
    subtask_description: &str,
    repeated_tool: &str,
    repeat_count: usize,
    recent_tool_calls: &str,
    context: &MentorPromptContext,
) -> String {
    format!(
        r#"The agent is working on: {description}
{context}

It has called '{tool}' {count} times. This suggests it may be stuck.

Recent tool calls:
{recent}

What should the agent do differently while staying inside the execution boundary?"#,
        description = subtask_description,
        context = render_mentor_context(context),
        tool = repeated_tool,
        count = repeat_count,
        recent = safe_truncate(recent_tool_calls, 3000),
    )
}

/// Enricher system prompt — gathers supplementary context before subtask execution.
///
/// Mirrors PentAGI's `enricher.tmpl`: searches memory, knowledge base, and past
/// results to add context that the executing agent wouldn't otherwise have.
pub fn enricher_system_prompt() -> &'static str {
    r#"You are a context enrichment specialist for a penetration testing / security engineering platform.

## YOUR ROLE

Before a subtask is delegated to a specialist agent, you gather SUPPLEMENTARY context
that will help the agent execute more effectively. You do NOT answer the question or
solve the task — you only retrieve additional relevant information.

## WHAT THE AGENT ALREADY RECEIVES

The specialist agent will automatically receive:
- The subtask title and description
- Execution context (completed subtask results, remaining plan)
- Its own system prompt and tools

Your enrichment result is injected as ADDITIONAL context alongside the task assignment.

## ENRICHMENT PROTOCOL

1. Check if completed subtasks contain findings relevant to this subtask
2. Identify dependencies — does this subtask need specific outputs from earlier ones?
3. Extract concrete technical details (IPs, ports, services, URLs, credentials) discovered so far
4. Note any failures or dead ends the agent should avoid repeating
5. If no additional context is needed, return "No additional context required."

## RULES

- Provide ONLY facts and data, NOT advice or solutions
- Do NOT repeat the subtask description — the agent already has it
- Keep enrichment concise (under 300 words)
- Focus on actionable intelligence: specific findings, URLs, credentials, tool outputs
- If previous subtasks found nothing relevant, say so briefly

## OUTPUT FORMAT

Return a concise enrichment block that will be prepended to the agent's task.
Use structured format:

**Relevant Findings**: [from completed subtasks]
**Dependencies**: [outputs this subtask needs]
**Avoid**: [known dead ends or failures]"#
}

/// Enricher user prompt — wraps the subtask and execution context for enrichment.
pub fn enricher_user_prompt(
    subtask_title: &str,
    subtask_description: &str,
    agent_type: &str,
    execution_context_summary: &str,
) -> String {
    let mut prompt = format!(
        r#"Enrich the following subtask for a {agent_type} agent:

<subtask>
<title>{title}</title>
<description>{description}</description>
</subtask>"#,
        agent_type = agent_type,
        title = subtask_title,
        description = subtask_description,
    );

    if !execution_context_summary.is_empty() {
        prompt.push_str(&format!(
            "\n\n<completed_work>\n{}\n</completed_work>",
            safe_truncate(execution_context_summary, 4000)
        ));
    } else {
        prompt.push_str("\n\n<completed_work>No completed subtasks yet. This is the first subtask.</completed_work>");
    }

    prompt.push_str("\n\nProvide supplementary context for the agent.");
    prompt
}

/// Wrap a subtask description with an execution plan (PentAGI's `task_assignment_wrapper.tmpl`).
pub fn wrap_task_with_plan(original_request: &str, execution_plan: &str) -> String {
    format!(
        r#"<task_assignment>
<original_request>
{request}
</original_request>

<execution_plan>
{plan}
</execution_plan>

<hint>
The original_request is the primary objective.
The execution_plan above was prepared by analyzing the broader context and decomposing the task into suggested steps.
Use this plan as guidance to work efficiently, but adapt your actions to the actual circumstances while staying aligned with the objective.
</hint>
</task_assignment>"#,
        request = original_request,
        plan = execution_plan,
    )
}

/// Task Planner system prompt — generates an execution plan before subtask starts.
pub fn task_planner_system_prompt() -> &'static str {
    r#"You are a planning adviser for specialized agents in a penetration testing / security engineering platform.

Your job: given a task assignment, produce a concise execution checklist (3-7 steps) the agent should follow.

## RULES

- Steps must be specific and actionable
- Include what to check or verify at each stage
- Highlight potential pitfalls the agent should avoid
- Keep the agent focused on the current task without scope creep
- Guide toward efficient completion without unnecessary actions
- Terminal commands execute independently (no persistent state between calls)

## OUTPUT

A numbered checklist, like:
1. [First critical action/verification step]
2. [Second step with specific details]
3. [Continue with remaining steps...]

No other text. No markdown fences. Just the numbered checklist."#
}

/// Task Planner user prompt — wraps the subtask for the planner.
pub fn task_planner_user_prompt(
    agent_type: &str,
    subtask_title: &str,
    subtask_description: &str,
    execution_context_summary: &str,
) -> String {
    let mut prompt = format!(
        r#"I am a {agent_type} agent and I have just received the following task assignment:

<my_task>
Title: {title}
Description: {description}
</my_task>"#,
        agent_type = agent_type,
        title = subtask_title,
        description = subtask_description,
    );

    if !execution_context_summary.is_empty() {
        prompt.push_str(&format!(
            "\n\n<completed_work>\n{}\n</completed_work>",
            safe_truncate(execution_context_summary, 3000)
        ));
    }

    prompt.push_str("\n\nCreate my execution plan checklist.");
    prompt
}

/// Reflector user prompt — wraps the agent's text response for the reflector.
pub fn reflector_user_prompt(subtask_title: &str, agent_response: &str) -> String {
    format!(
        r#"The agent was working on subtask "{title}" but responded with text instead of using tools:

---
{response}
---

Redirect it to take concrete action. What specific tool should it use first?"#,
        title = subtask_title,
        response = safe_truncate(agent_response, 2000),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_intel_mentor_objective_preserves_adaptive_goal_semantics() {
        let guard = mentor_hard_stage_guard(&MentorPromptContext {
            stage: Some("target_intel".to_string()),
            agent_role: Some("goal_owner".to_string()),
            allowed_tools: vec![
                "recon_search_intel".to_string(),
                "stage_team_request_intel_review".to_string(),
            ],
            allowed_tool_types: Vec::new(),
        });

        assert!(guard.contains("adaptive corporate-intelligence Goal"));
        assert!(guard.contains("high-information semantic pivots"));
        assert!(guard.contains("material frontier"));
        assert!(guard.contains("request neutral review"));
        assert!(!guard.contains("provider/source evidence"));
        assert!(!guard.contains("recon_lookup_whois"));
    }
}
