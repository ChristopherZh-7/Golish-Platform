use std::sync::Arc;

use rig::completion::request::ToolDefinition;
use tokio::sync::mpsc;
use tokio::sync::RwLock;

use golish_core::events::AiEvent;
use golish_core::ApiRequestStats;
use golish_tools::ToolRegistry;

/// Barrier tool name used by all sub-agents to submit structured results.
/// When a sub-agent calls this tool, the executor terminates the loop and
/// returns the structured result to the parent agent (PentAGI barrier pattern).
pub const BARRIER_TOOL_NAME: &str = "submit_result";

/// Trait for providing tool definitions to the sub-agent executor.
/// This allows the executor to be decoupled from the tool definition source.
#[async_trait::async_trait]
pub trait ToolProvider: Send + Sync {
    /// Get all available tool definitions
    fn get_all_tool_definitions(&self) -> Vec<ToolDefinition>;

    /// Filter tools to only those allowed by the sub-agent
    fn filter_tools_by_allowed(
        &self,
        tools: Vec<ToolDefinition>,
        allowed: &[String],
    ) -> Vec<ToolDefinition>;

    /// Execute a web fetch tool
    async fn execute_web_fetch_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> (serde_json::Value, bool);

    async fn execute_memory_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)>;

    /// Execute a knowledge-base / vulnerability-wiki tool (`search_knowledge_base`,
    /// `read_knowledge`, `write_knowledge`, `ingest_cve`, `save_poc`, …). Returns
    /// `None` when `tool_name` isn't a KB tool so the caller falls through to the
    /// router / registry — same contract as [`Self::execute_memory_tool`].
    async fn execute_knowledge_base_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(serde_json::Value, bool)>;

    /// Normalize run_pty_cmd arguments
    fn normalize_run_pty_cmd_args(&self, args: serde_json::Value) -> serde_json::Value;
}

/// Trait for sub-agent chain persistence operations (decoupled from sqlx).
///
/// Provides the message-chain DB operations the executor needs to
/// save/restore persistent chains across sub-agent invocations.
#[async_trait::async_trait]
pub trait SubAgentChainPersistence: Send + Sync {
    async fn chain_create(
        &self,
        session_id: uuid::Uuid,
        task_id: Option<uuid::Uuid>,
        subtask_id: Option<uuid::Uuid>,
        agent_type: &str,
        parent_chain_id: Option<uuid::Uuid>,
        model: Option<&str>,
    ) -> anyhow::Result<uuid::Uuid>;

    async fn chain_update(
        &self,
        id: uuid::Uuid,
        chain_json: &serde_json::Value,
    ) -> anyhow::Result<()>;

    async fn chain_update_usage(
        &self,
        id: uuid::Uuid,
        input_tokens: i32,
        output_tokens: i32,
        cache_read_tokens: i32,
        input_cost: f64,
        output_cost: f64,
        duration_ms: i32,
    ) -> anyhow::Result<()>;

    /// Load the most recent persisted chain for `(session, agent)`, if any, so a
    /// `resume` delegation can replay the same sub-agent's prior conversation
    /// (including the tool results / evidence ids it produced). Returns
    /// `(chain_id, chain_json)`. Default impl: no prior chain (never resumes).
    async fn chain_load_latest(
        &self,
        _session_id: uuid::Uuid,
        _task_id: Option<uuid::Uuid>,
        _agent_type: &str,
    ) -> anyhow::Result<Option<(uuid::Uuid, serde_json::Value)>> {
        Ok(None)
    }

    /// Load a SPECIFIC persisted chain by its id, so a `resume` delegation can
    /// continue an exact prior sub-agent conversation. The id is handed back to
    /// the orchestrator when the sub-agent finishes, so it can name precisely
    /// which worker to resume. Default impl: not found.
    async fn chain_load_by_id(
        &self,
        _chain_id: uuid::Uuid,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(None)
    }

    async fn load_prompt_template_overrides(&self) -> Vec<(String, String)>;
}

/// Async callback invoked after a shell command completes.
///
/// Arguments: (command, stdout, project_path).
/// The closure captures external resources (e.g. a DB pool) it needs.
pub type PostShellHook = Arc<
    dyn Fn(
            String,
            String,
            Option<String>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Extra tool executor for tools that live OUTSIDE the [`ToolRegistry`]
/// (graph / memory). Given `(tool_name, args)` it returns
/// `Some((value, success))` if it handled the call, else `None` to fall through
/// to the registry. A plain `Fn` so this crate stays free of harness-crate
/// deps; the runtime builds it with the graph/memory backends wired in.
pub type SubAgentToolRouter = Arc<
    dyn Fn(
            String,
            serde_json::Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<(serde_json::Value, bool)>> + Send>,
        > + Send
        + Sync,
>;

/// Per-stage tool-boundary guard for sub-agent tool calls.
///
/// Given `(tool_name, args)` it returns `Err(reason)` when the call is NOT
/// allowed in the active harness stage (deny-by-default category whitelist —
/// see `docs/design/2026-06-02-stage-tool-whitelist-enforcement.md`). The
/// sub-agent executor calls it before running each tool and turns `Err` into a
/// synthetic error result instead of executing.
///
/// A plain `Fn` so this crate stays free of harness-crate deps; the runtime
/// builds it from the stage's `allowed_tool_types`.
pub type StageToolGuard = Arc<dyn Fn(&str, &serde_json::Value) -> Result<(), String> + Send + Sync>;

/// Per-stage tool-list filter controlling tool *visibility* (D1).
///
/// Returns `true` when a tool name should be HIDDEN from the sub-agent's tool
/// list for the active harness stage (e.g. scan tools during a no-scan stage
/// like `scoping`). Mirrors the main agent's `hide_scans_for_zero_scan_stage`
/// so a delegated sub-agent never even *sees* a tool it could only be denied —
/// which stops the retry spin where the model hammers a blocked tool. The
/// call-time [`StageToolGuard`] remains the backstop.
///
/// A plain `Fn` so this crate stays free of harness-crate deps; the runtime
/// builds it from the stage's `allowed_tool_types`.
pub type StageToolHider = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Context needed for sub-agent execution.
pub struct SubAgentExecutorContext<'a> {
    pub event_tx: &'a mpsc::UnboundedSender<AiEvent>,
    pub tool_registry: &'a Arc<RwLock<ToolRegistry>>,
    pub workspace: &'a Arc<RwLock<std::path::PathBuf>>,
    /// Provider name (e.g., "openai", "anthropic_vertex") for model capability checks
    pub provider_name: &'a str,
    /// Model name for model capability checks
    pub model_name: &'a str,
    /// Session ID for Langfuse tracing (propagated from parent agent)
    pub session_id: Option<&'a str>,
    /// Base directory for transcript files (e.g., `~/.golish/transcripts`)
    /// If set, sub-agent internal events will be written to separate transcript files.
    pub transcript_base_dir: Option<&'a std::path::Path>,
    /// API request stats collector (per session, optional)
    pub api_request_stats: Option<&'a Arc<ApiRequestStats>>,
    /// Orchestrator briefing injected before execution. Contains relevant memories,
    /// execution plan context, and findings from other agents. Appended to the
    /// effective system prompt as a `## Briefing from Orchestrator` section.
    pub briefing: Option<String>,
    /// Per-agent temperature override from settings (None = use default 0.3).
    pub temperature_override: Option<f32>,
    /// Per-agent max_tokens override from settings (None = use default 8192).
    pub max_tokens_override: Option<u32>,
    /// Per-agent top_p override from settings (None = not sent to provider).
    pub top_p_override: Option<f32>,
    /// Chain persistence backend for saving/restoring sub-agent conversation
    /// chains (PentAGI-style). Replaces the raw `sqlx::PgPool`.
    pub chain_persistence: Option<&'a Arc<dyn SubAgentChainPersistence>>,
    /// Sub-agent registry for nested delegation (PentAGI hierarchical pattern).
    /// When set, agents with `delegatable_agents` can invoke other sub-agents.
    pub sub_agent_registry: Option<&'a Arc<RwLock<crate::definition::SubAgentRegistry>>>,
    /// Optional hook called after a successful shell tool execution.
    pub post_shell_hook: Option<PostShellHook>,
    /// AI-controlled resume handle for this delegation:
    /// `Some("<chain_id>")` continues that exact prior sub-agent conversation;
    /// `Some("latest")` continues this agent's most recent chain; `None` is a
    /// fresh sub-agent (default). Enables "go back to the same worker".
    pub resume: Option<String>,
    /// Extra tool executor for tools outside the `ToolRegistry` (graph/memory),
    /// tried before the registry fallback so a delegated sub-agent can actually
    /// run them (e.g. `graph_add_entity`) instead of getting "Unknown tool".
    pub sub_tool_router: Option<SubAgentToolRouter>,
    /// Optional per-stage tool boundary guard (forbidden-only). When the
    /// sub-agent runs inside a harness stage, this blocks tool calls whose
    /// resolved capability is in the stage's forbidden list (e.g. `dig` in
    /// scoping), before execution. `None` = no active stage (legacy behaviour).
    pub stage_tool_guard: Option<StageToolGuard>,
    /// Optional per-stage tool-list filter (D1). When the sub-agent runs inside
    /// a harness stage that permits no scan tools (e.g. `scoping`), this hides
    /// scan tools from the exposed list so the model never even attempts one —
    /// preventing the retry spin the call-time guard alone can't stop. `None` =
    /// no filtering (legacy behaviour).
    pub hide_tool_in_stage: Option<StageToolHider>,
}
