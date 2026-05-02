use golish_llm_providers::ModelCapabilities;

pub struct AgenticLoopConfig {
    /// Model capabilities (thinking support, temperature, etc.)
    pub capabilities: ModelCapabilities,
    /// Whether HITL approval is required for tool execution.
    pub require_hitl: bool,
    /// Whether this is a sub-agent execution (affects tool restrictions).
    pub is_sub_agent: bool,
    /// Whether to invoke the reflector agent when the model produces text
    /// without any tool calls. Default: false (only enabled for main agent).
    pub enable_reflector: bool,
    /// Tool names hint passed to the reflector so it can suggest specific tools.
    pub tool_names_for_reflector: Option<Vec<String>>,
}

impl AgenticLoopConfig {
    /// Create config for main agent with Anthropic model.
    ///
    /// Anthropic models support extended thinking (reasoning history tracking)
    /// and require HITL approval for tool execution.
    pub fn main_agent_anthropic() -> Self {
        Self {
            capabilities: ModelCapabilities::anthropic_defaults(),
            require_hitl: true,
            is_sub_agent: false,
            enable_reflector: true,
            tool_names_for_reflector: None,
        }
    }

    /// Create config for main agent with generic model.
    ///
    /// Generic models use conservative defaults (no thinking history tracking)
    /// and require HITL approval for tool execution.
    pub fn main_agent_generic() -> Self {
        Self {
            capabilities: ModelCapabilities::conservative_defaults(),
            require_hitl: true,
            is_sub_agent: false,
            enable_reflector: true,
            tool_names_for_reflector: None,
        }
    }

    /// Create config for sub-agent (trusted, no HITL).
    ///
    /// Sub-agents are trusted and do not require HITL approval.
    /// The capabilities should match the model being used.
    pub fn sub_agent(capabilities: ModelCapabilities) -> Self {
        Self {
            capabilities,
            require_hitl: false,
            is_sub_agent: true,
            enable_reflector: false,
            tool_names_for_reflector: None,
        }
    }

    /// Create config with detected capabilities based on provider and model name.
    ///
    /// This factory method detects capabilities automatically and is useful
    /// when calling from code that has provider/model info but not an LlmClient.
    pub fn with_detection(provider_name: &str, model_name: &str, is_sub_agent: bool) -> Self {
        Self {
            capabilities: ModelCapabilities::detect(provider_name, model_name),
            require_hitl: !is_sub_agent,
            is_sub_agent,
            enable_reflector: !is_sub_agent,
            tool_names_for_reflector: None,
        }
    }
}
