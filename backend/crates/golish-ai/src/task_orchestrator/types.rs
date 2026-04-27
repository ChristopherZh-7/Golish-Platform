//! Public types and the [`AgentExecutor`] trait used by [`TaskOrchestrator`].
//!
//! Includes the planning DTOs (`PlannedSubtask`, `GeneratorOutput`,
//! `RefinerOutput`, `SubtaskModification`), per-call cost tracking
//! (`AgentTokenUsage`, `TaskCostTracker`), execution context types
//! (`ExecutionContext`, `SubtaskResult`, `AgentResult`), and the
//! [`AgentExecutor`] callback trait that decouples the orchestrator from
//! `AgentBridge`.

use serde::{Deserialize, Serialize};

use anyhow::Result;

/// Maximum number of subtasks per task (safety limit matching PentAGI's TasksNumberLimit+3).
pub(super) const MAX_SUBTASKS: usize = 13;
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
}

/// The generator's response — a list of subtasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorOutput {
    pub subtasks: Vec<PlannedSubtask>,
}

/// The refiner's response — structured patch operations on the remaining plan.
///
/// Mirrors PentAGI's `SubtaskPatch` pattern: instead of regenerating the entire plan,
/// the refiner applies surgical operations (add/remove/modify/reorder) which is
/// more token-efficient and preserves context better.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinerOutput {
    /// Subtasks to add to the queue.
    #[serde(default)]
    pub add: Vec<PlannedSubtask>,
    /// Indices (0-based, relative to remaining queue) to remove.
    #[serde(default)]
    pub remove: Vec<usize>,
    /// Modifications to apply to existing subtasks (by 0-based index in remaining queue).
    #[serde(default)]
    pub modify: Vec<SubtaskModification>,
    /// New ordering of remaining subtasks (0-based indices). If provided, subtasks
    /// are reordered accordingly before add/remove operations.
    #[serde(default)]
    pub reorder: Option<Vec<usize>>,
    /// Whether the task is considered complete (skip remaining subtasks).
    #[serde(default)]
    pub complete: bool,
}

/// A modification to an existing subtask in the remaining queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskModification {
    /// 0-based index in the remaining queue.
    pub index: usize,
    /// New title (if changed).
    #[serde(default)]
    pub title: Option<String>,
    /// New description (if changed).
    #[serde(default)]
    pub description: Option<String>,
    /// New agent assignment (if changed).
    #[serde(default)]
    pub agent: Option<String>,
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

/// Accumulated cost tracking across all agent calls in a task.
#[derive(Debug, Clone, Default)]
pub struct TaskCostTracker {
    pub entries: Vec<AgentTokenUsage>,
}

impl TaskCostTracker {
    pub fn record(&mut self, entry: AgentTokenUsage) {
        self.entries.push(entry);
    }

    pub fn total_input_tokens(&self) -> u64 {
        self.entries.iter().map(|e| e.input_tokens).sum()
    }

    pub fn total_output_tokens(&self) -> u64 {
        self.entries.iter().map(|e| e.output_tokens).sum()
    }

    pub fn total_duration_ms(&self) -> u64 {
        self.entries.iter().map(|e| e.duration_ms).sum()
    }
}

/// Context accumulated during task execution, passed between agents.
#[derive(Debug, Clone, Default)]
pub struct ExecutionContext {
    /// Accumulated results from completed subtasks.
    pub completed_results: Vec<SubtaskResult>,
    /// The original user input.
    pub task_input: String,
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
    /// Run the generator to decompose the task into subtasks.
    async fn generate_subtasks(
        &self,
        task_input: &str,
    ) -> Result<GeneratorOutput>;

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

    /// Run the refiner to adjust the remaining plan.
    async fn refine_plan(
        &self,
        execution_context: &ExecutionContext,
        remaining_subtasks: &[PlannedSubtask],
    ) -> Result<RefinerOutput>;

    /// Run the reporter to generate the final summary.
    async fn generate_report(
        &self,
        execution_context: &ExecutionContext,
    ) -> Result<AgentResult>;

    /// Run the reflector to redirect an agent that returned plain text.
    ///
    /// Returns a corrective message that should be injected as a user message
    /// before retrying the subtask. The reflector acts as a "proxy user" that
    /// guides the agent back to tool usage (PentAGI's Reflector pattern).
    async fn reflect(
        &self,
        subtask_title: &str,
        agent_response: &str,
    ) -> Result<String>;

    /// Serialize the current message chain for persistence.
    ///
    /// Returns the conversation messages as JSON for storage in the
    /// `message_chains` table. Default returns `None` (no persistence).
    fn current_message_chain(&self) -> Option<serde_json::Value> {
        None
    }
}

