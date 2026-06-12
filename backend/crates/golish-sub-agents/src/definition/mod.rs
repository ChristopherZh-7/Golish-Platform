//! Sub-agent definitions, context, and registry.
//!
//! This module provides the infrastructure for:
//! - Defining specialized sub-agents with custom system prompts and tool restrictions
//! - Managing state and context between agents
//! - Registering and retrieving sub-agent definitions

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where a sub-agent definition was loaded from
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "path")]
pub enum AgentSource {
    /// Hardcoded in Rust (system-level agents: worker, memorist, reflector)
    #[default]
    BuiltIn,
    /// Loaded from a .md file on disk
    File(PathBuf),
}

/// Context passed to a sub-agent during execution
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubAgentContext {
    /// The original user request that triggered the workflow
    pub original_request: String,

    /// Summary of conversation history for context awareness
    pub conversation_summary: Option<String>,

    /// Variables passed from parent agent's state
    pub variables: HashMap<String, serde_json::Value>,

    /// Current depth in the agent hierarchy (to prevent infinite recursion)
    pub depth: usize,

    /// Which agent delegated to this one (e.g. "main-agent", "pentester")
    #[serde(default)]
    pub parent_agent: Option<String>,

    /// Associated task ID for multi-step plans
    #[serde(default)]
    pub task_id: Option<String>,

    /// Associated subtask ID within a plan
    #[serde(default)]
    pub subtask_id: Option<String>,

    /// Summaries of previously completed subtasks in the current plan,
    /// giving this agent awareness of what has already been done.
    #[serde(default)]
    pub execution_history: Vec<String>,
}

/// Result returned by a sub-agent after execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentResult {
    /// ID of the sub-agent that produced this result
    pub agent_id: String,

    /// The agent's response text
    pub response: String,

    /// Updated context (may include new variables)
    pub context: SubAgentContext,

    /// Whether the sub-agent completed successfully
    pub success: bool,

    /// Execution duration in milliseconds
    pub duration_ms: u64,

    /// Files modified by this sub-agent during execution
    #[serde(default)]
    pub files_modified: Vec<String>,
}

/// Definition of a specialized sub-agent
#[derive(Clone, Debug)]
pub struct SubAgentDefinition {
    /// Unique identifier for this sub-agent
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Description for the parent agent to understand when to invoke this sub-agent
    pub description: String,

    /// System prompt that defines this sub-agent's role and capabilities
    pub system_prompt: String,

    /// List of tool names this sub-agent is allowed to use (empty = all tools)
    pub allowed_tools: Vec<String>,

    /// Maximum iterations for this sub-agent's tool loop
    pub max_iterations: usize,

    /// Optional model override (provider_name, model_name).
    /// When set, this sub-agent uses a different model than the main agent.
    /// None = inherit the main agent's model.
    pub model_override: Option<(String, String)>,

    /// Overall (wall-clock) timeout for the entire sub-agent execution in
    /// seconds. `None` = no overall timeout: the agent runs to completion,
    /// bounded only by `idle_timeout_secs` and `max_iterations`. Default: `None`.
    pub timeout_secs: Option<u64>,

    /// Idle timeout - max seconds without any progress (LLM chunk, tool result).
    /// None = no idle timeout. Default: 180 (3 minutes).
    pub idle_timeout_secs: Option<u64>,

    /// Optional prompt generation system prompt. When set, the executor makes an LLM call
    /// using this as the system prompt and the task/context as the user message to generate
    /// the sub-agent's system prompt before execution. The definition's `system_prompt`
    /// field is used as a fallback if prompt generation fails.
    /// When `None`, the `system_prompt` is used directly (default for specialized agents).
    pub prompt_template: Option<String>,

    /// Per-agent temperature override from settings. None = use default.
    pub temperature: Option<f32>,

    /// Per-agent max_tokens override from settings. None = use default.
    pub max_tokens: Option<u32>,

    /// Per-agent top_p override from settings. None = not sent to provider.
    pub top_p: Option<f32>,

    /// Where this definition was loaded from
    pub source: AgentSource,

    /// System-level agents cannot be deleted from the UI.
    /// They are critical for the runtime (worker, memorist, reflector).
    pub is_system: bool,

    /// If true, runs in read-only mode (no file writes, no state-changing commands)
    pub readonly: bool,

    /// If true, runs in background without blocking the parent agent
    pub is_background: bool,

    /// IDs of other sub-agents this agent can delegate to (nested delegation).
    /// When non-empty, the executor injects `sub_agent_{id}` tools for each listed agent
    /// and handles recursive dispatch. Matches PentAGI's hierarchical pattern
    /// (e.g., pentester can delegate to coder, searcher).
    pub delegatable_agents: Vec<String>,

    /// Pipeline-only agents are used internally by the task orchestrator
    /// (e.g., reflector, refiner) and should NOT appear as delegatable
    /// `sub_agent_*` tools. They still exist in the registry for prompt
    /// and config lookup but are filtered from tool generation.
    pub pipeline_only: bool,
}

impl SubAgentDefinition {
    /// Create a new sub-agent definition
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        system_prompt: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            system_prompt: system_prompt.into(),
            allowed_tools: Vec::new(),
            max_iterations: 50,
            model_override: None,
            timeout_secs: None,
            idle_timeout_secs: Some(180),
            prompt_template: None,
            temperature: None,
            max_tokens: None,
            top_p: None,
            source: AgentSource::BuiltIn,
            is_system: false,
            readonly: false,
            is_background: false,
            delegatable_agents: Vec::new(),
            pipeline_only: false,
        }
    }

    /// Mark as pipeline-only (not exposed as `sub_agent_*` tool).
    pub fn as_pipeline_only(mut self) -> Self {
        self.pipeline_only = true;
        self
    }

    /// Set which sub-agents this agent can delegate to (nested delegation).
    pub fn with_delegatable_agents(mut self, agents: Vec<String>) -> Self {
        self.delegatable_agents = agents;
        self
    }

    /// Mark this agent as a system agent (cannot be deleted from UI)
    pub fn as_system(mut self) -> Self {
        self.is_system = true;
        self
    }

    /// Mark this agent as read-only
    pub fn with_readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    /// Mark this agent as background-capable
    pub fn with_background(mut self, is_background: bool) -> Self {
        self.is_background = is_background;
        self
    }

    /// Set allowed tools for this sub-agent
    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.allowed_tools = tools;
        self
    }

    /// Set a prompt generation system prompt. When set, the executor uses this as the
    /// system prompt in an LLM call (with task/context as user message) to generate
    /// an optimized system prompt for the sub-agent before execution.
    pub fn with_prompt_template(mut self, template: impl Into<String>) -> Self {
        self.prompt_template = Some(template.into());
        self
    }

    /// Set maximum iterations
    pub fn with_max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = max;
        self
    }

    /// Set overall timeout in seconds
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Set idle timeout in seconds
    pub fn with_idle_timeout(mut self, secs: u64) -> Self {
        self.idle_timeout_secs = Some(secs);
        self
    }

    /// Set model override for this sub-agent (builder pattern)
    pub fn with_model_override(
        mut self,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        self.model_override = Some((provider.into(), model.into()));
        self
    }

    /// Set model override at runtime
    pub fn set_model_override(&mut self, provider: impl Into<String>, model: impl Into<String>) {
        self.model_override = Some((provider.into(), model.into()));
    }

    /// Clear model override (will inherit main agent's model)
    pub fn clear_model_override(&mut self) {
        self.model_override = None;
    }
}

/// Registry of available sub-agents
#[derive(Default)]
pub struct SubAgentRegistry {
    agents: HashMap<String, SubAgentDefinition>,
}

impl SubAgentRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    /// Get a sub-agent by ID
    pub fn get(&self, id: &str) -> Option<&SubAgentDefinition> {
        self.agents.get(id)
    }

    /// Get a mutable reference to a sub-agent by ID
    pub fn get_mut(&mut self, id: &str) -> Option<&mut SubAgentDefinition> {
        self.agents.get_mut(id)
    }

    /// Get all registered sub-agents
    pub fn all(&self) -> impl Iterator<Item = &SubAgentDefinition> {
        self.agents.values()
    }

    /// Get count of registered sub-agents
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.agents.len()
    }

    /// Check if registry is empty
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    /// Register a sub-agent in the registry
    pub fn register(&mut self, agent: SubAgentDefinition) {
        self.agents.insert(agent.id.clone(), agent);
    }

    /// Register multiple sub-agents at once
    pub fn register_multiple(&mut self, agents: Vec<SubAgentDefinition>) {
        for agent in agents {
            self.register(agent);
        }
    }

    /// Re-register definitions while preserving the per-agent runtime overrides
    /// applied after construction (model routing + LLM params).
    ///
    /// The DB-template reload (`AgentBridge::set_db_backend`) rebuilds
    /// definitions from defaults — which carry no `model_override` /
    /// `temperature` / `max_tokens` / `top_p` — so a plain
    /// [`Self::register_multiple`] would wipe overrides set by
    /// `apply_sub_agent_model_settings` (startup) or the `set_sub_agent_model`
    /// command (runtime). The reload's only purpose is to refresh `system_prompt`
    /// from DB templates, so it must carry those four fields over from the
    /// existing entry (matched by id). Race-safe: the caller holds the registry
    /// write lock across this whole call, and the only other writers of those
    /// fields take the same lock, so the read-existing-then-insert is atomic.
    pub fn register_preserving_overrides(&mut self, agents: Vec<SubAgentDefinition>) {
        for mut agent in agents {
            if let Some(existing) = self.agents.get(&agent.id) {
                agent.model_override = existing.model_override.clone();
                agent.temperature = existing.temperature;
                agent.max_tokens = existing.max_tokens;
                agent.top_p = existing.top_p;
            }
            self.register(agent);
        }
    }
}

/// Maximum sub-agent recursion depth.
///
/// `2` caps nesting at a single level: the primary agent (depth 0) may dispatch
/// sub-agents (depth 1), but sub-agents may NOT spawn further sub-agents
/// (depth ≥ 1 gets no dispatch / delegation tools, and the bridge hard-errors at
/// depth ≥ 2 as a backstop). This kills the wasteful same-type recursion observed
/// in practice (pentester → pentester → pentester). NOTE: it also disables
/// hierarchical delegation (e.g. pentester → coder/searcher); raise to `3` to
/// allow exactly one delegation level if that is wanted.
pub const MAX_AGENT_DEPTH: usize = 2;
