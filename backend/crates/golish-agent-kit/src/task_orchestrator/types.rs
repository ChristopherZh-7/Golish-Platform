//! Public types and the [`AgentExecutor`] trait used by [`TaskOrchestrator`].
//!
//! Includes the planning DTO (`PlannedSubtask`), per-call token usage
//! (`AgentTokenUsage`), execution context types (`ExecutionContext`,
//! `SubtaskResult`, `AgentResult`), and the [`AgentExecutor`] callback trait
//! that decouples the orchestrator from `AgentBridge`.

use serde::{Deserialize, Serialize};

use anyhow::Result;

/// Maximum reflector attempts before giving up (matches PentAGI's maxReflectorCallsPerChain).
pub(super) const MAX_REFLECTOR_RETRIES: usize = 3;

/// A planned subtask from the Generator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedSubtask {
    pub title: String,
    pub description: String,
    /// Which specialist should handle this (e.g. "pentester", "coder").
    /// The primary agent uses this as guidance, not a hard constraint.
    pub agent: Option<String>,
    /// Doc 3 §5.2 stage harness hint · 当 subtask 归属某 stage 时填入.
    /// `None` → subtask 不挂任何 stage (gate hook 透传, 不推进游标).
    /// `Some(_)` → execute_single_subtask 末端 hook 走 StageHarness validate_gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_stage: Option<crate::harness::HarnessStageHint>,
    /// Doc 3 §6 NlSlice (终态 4 字段) · stage 内 inner loop 用.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nl_slice: Option<crate::harness::NlSlice>,
    /// 自由文本验收标准 · gate validator 之外的 soft acceptance.
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

/// Token usage statistics for a single agent call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Agent phase that consumed these tokens (e.g. "generator", "primary_agent", "refiner", "reporter").
    pub phase: String,
}

/// Context accumulated during task execution, passed between agents.
///
/// Renders in PentAGI-compatible XML format for injection into agent prompts.
#[derive(Debug, Clone, Default)]
pub struct ExecutionContext {
    /// Accumulated results from completed subtasks.
    pub completed_results: Vec<SubtaskResult>,
    /// The original user input.
    pub task_input: String,
    /// Current subtask being executed (if any).
    pub current_subtask: Option<CurrentSubtask>,
    /// Remaining planned subtasks (after the current one).
    pub planned_subtasks: Vec<PlannedSubtaskInfo>,
    /// C3 · harness stage of the current subtask (when stage_mode on). Threaded
    /// to the bridge → agentic loop so per-tool dispatch can enforce the stage's
    /// forbidden-tool barrier. `None` = no stage / flag off.
    pub harness_stage: Option<crate::harness::StageKind>,
    /// C3 · authorization context (profile ceiling + classified intent) for the
    /// current subtask. Threaded alongside `harness_stage` so per-tool dispatch
    /// can run the full pre-action authorizer (allowed_tools confinement + intent
    /// vs ceiling) on real executor tools. `None` = no stage / flag off.
    pub harness_authz: Option<crate::harness::HarnessAuthz>,
    /// C1 · the operation's profile id (from `operation_state.profile`, e.g.
    /// "assessment" / "pentest"). Threaded so the gate hook constructs the
    /// `StageHarness` with the real profile instead of a hardcoded placeholder.
    /// `None` = flag off / no operation_state row (hook falls back to "assessment").
    pub harness_profile_id: Option<String>,
}

/// Info about a subtask being currently executed.
#[derive(Debug, Clone)]
pub struct CurrentSubtask {
    pub title: String,
    pub description: String,
    pub agent: Option<String>,
}

/// Lightweight info about a planned subtask for context display.
#[derive(Debug, Clone)]
pub struct PlannedSubtaskInfo {
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct SubtaskResult {
    pub title: String,
    pub result: String,
    /// Token usage for executing this subtask (if tracked).
    pub token_usage: Option<AgentTokenUsage>,
}

impl ExecutionContext {
    pub fn summary(&self) -> String {
        if self.completed_results.is_empty() {
            return "No subtasks completed yet.".to_string();
        }
        let mut s = String::new();
        for (i, r) in self.completed_results.iter().enumerate() {
            s.push_str(&format!(
                "### Subtask {} — {}\n{}\n\n",
                i + 1,
                r.title,
                r.result
            ));
        }
        s
    }

    /// Render the execution context in PentAGI-compatible XML format.
    ///
    /// This format is injected into the orchestrator's prompt as `{{execution_context}}`.
    pub fn render_xml(&self) -> String {
        let mut out = String::new();

        out.push_str("<global_task>\n");
        out.push_str(&self.task_input);
        out.push_str("\n</global_task>\n\n");

        out.push_str("<completed_subtasks>\n");
        if self.completed_results.is_empty() {
            out.push_str("<status>none</status>\n");
            out.push_str(
                "<message>No completed subtasks yet. This is the first subtask.</message>\n",
            );
        } else {
            for (i, r) in self.completed_results.iter().enumerate() {
                out.push_str(&format!(
                    "<subtask>\n<index>{}</index>\n<title>{}</title>\n<result>{}</result>\n</subtask>\n",
                    i + 1, r.title, r.result
                ));
            }
        }
        out.push_str("</completed_subtasks>\n\n");

        if let Some(ref current) = self.current_subtask {
            out.push_str("<current_subtask>\n");
            out.push_str(&format!("<title>{}</title>\n", current.title));
            out.push_str(&format!(
                "<description>{}</description>\n",
                current.description
            ));
            if let Some(ref agent) = current.agent {
                out.push_str(&format!("<assigned_agent>{}</assigned_agent>\n", agent));
            }
            out.push_str("</current_subtask>\n\n");
        }

        out.push_str("<planned_subtasks>\n");
        if self.planned_subtasks.is_empty() {
            out.push_str("<status>none</status>\n");
            out.push_str("<message>No remaining subtasks in the backlog.</message>\n");
        } else {
            for (i, p) in self.planned_subtasks.iter().enumerate() {
                out.push_str(&format!(
                    "<subtask>\n<index>{}</index>\n<title>{}</title>\n<description>{}</description>\n</subtask>\n",
                    i + 1, p.title, p.description
                ));
            }
        }
        out.push_str("</planned_subtasks>");

        out
    }
}

/// Result from an agent execution that includes token tracking.
#[derive(Debug, Clone)]
pub struct AgentResult {
    pub content: String,
    pub token_usage: Option<AgentTokenUsage>,
}

impl AgentResult {
    pub fn new(content: String) -> Self {
        Self {
            content,
            token_usage: None,
        }
    }

    pub fn with_usage(content: String, usage: AgentTokenUsage) -> Self {
        Self {
            content,
            token_usage: Some(usage),
        }
    }
}

/// Callback trait for the orchestrator to invoke LLM agents.
///
/// This decouples the orchestrator from `AgentBridge` directly,
/// making it testable and allowing different execution strategies.
///
/// All methods return `AgentResult` to enable per-call token tracking
/// (PentAGI-style per-chain cost accounting).
#[async_trait::async_trait]
pub trait AgentExecutor: Send + Sync {
    /// Execute a single subtask as the primary agent.
    /// Returns the result text and optional token usage.
    /// `agent_type` is the specialist type assigned by the Generator (e.g., "pentester", "coder").
    async fn execute_subtask(
        &self,
        subtask_title: &str,
        subtask_description: &str,
        execution_context: &ExecutionContext,
        agent_type: Option<&str>,
    ) -> Result<AgentResult>;

    /// Run the reporter to generate the final summary.
    async fn generate_report(&self, execution_context: &ExecutionContext) -> Result<AgentResult>;

    /// Run the reflector to redirect an agent that returned plain text.
    ///
    /// Returns a corrective message that should be injected as a user message
    /// before retrying the subtask. The reflector acts as a "proxy user" that
    /// guides the agent back to tool usage (PentAGI's Reflector pattern).
    async fn reflect(&self, subtask_title: &str, agent_response: &str) -> Result<String>;

    /// Enrich a subtask with supplementary context before execution.
    ///
    /// Mirrors PentAGI's `enricher.tmpl`: searches memory, knowledge base,
    /// and completed subtask results to add context the executing agent
    /// wouldn't otherwise have. Returns the enrichment text to prepend.
    ///
    /// Default returns `Ok(None)` (no enrichment).
    async fn enrich_subtask(
        &self,
        subtask_title: &str,
        subtask_description: &str,
        execution_context: &ExecutionContext,
        agent_type: &str,
    ) -> Result<Option<String>> {
        let _ = (
            subtask_title,
            subtask_description,
            execution_context,
            agent_type,
        );
        Ok(None)
    }

    /// Generate an execution plan for a subtask before delegating it.
    ///
    /// Mirrors PentAGI's `question_task_planner.tmpl` + `task_assignment_wrapper.tmpl`:
    /// the Adviser creates a concise checklist (3-7 steps) that is wrapped
    /// around the original task description.
    ///
    /// Default returns `Ok(None)` (no pre-planning).
    async fn plan_subtask(
        &self,
        subtask_title: &str,
        subtask_description: &str,
        execution_context: &ExecutionContext,
        agent_type: &str,
    ) -> Result<Option<String>> {
        let _ = (
            subtask_title,
            subtask_description,
            execution_context,
            agent_type,
        );
        Ok(None)
    }

    /// Monitor execution progress and provide corrective advice.
    ///
    /// Mirrors PentAGI's `question_execution_monitor.tmpl`: when the agentic
    /// loop detects repetitive tool usage, this method is called to generate
    /// strategic advice that is injected into the next tool response.
    ///
    /// Default returns `Ok(None)` (no advice).
    async fn monitor_execution(
        &self,
        subtask_description: &str,
        repeated_tool: &str,
        repeat_count: usize,
        recent_tool_calls: &str,
    ) -> Result<Option<String>> {
        let _ = (
            subtask_description,
            repeated_tool,
            repeat_count,
            recent_tool_calls,
        );
        Ok(None)
    }

    /// Serialize the current message chain for persistence.
    ///
    /// Returns the conversation messages as JSON for storage in the
    /// `message_chains` table. Default returns `None` (no persistence).
    fn current_message_chain(&self) -> Option<serde_json::Value> {
        None
    }
}
