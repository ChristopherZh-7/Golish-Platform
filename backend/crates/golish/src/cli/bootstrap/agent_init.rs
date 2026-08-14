//! Agent initialization and API key resolution helpers.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::ai::agent_bridge::AgentBridge;
use crate::indexer::IndexerState;
use crate::settings::{get_with_env_fallback, GolishSettings};
use crate::sidecar::SidecarState;
use golish_agent_app::ai::provider_bootstrap::{normalize_agent_bootstrap, AgentBootstrapConfig};
use golish_agent_kit::llm_client::ProviderConfig;
use golish_core::runtime::GolishRuntime;

use super::super::args::Args;

/// Initialize the AI agent bridge with all dependencies.
///
/// `event_session_id` becomes the bridge's event/evidence session identity: it
/// is stamped on every evidence-ledger row (`audit_log.session_id`), attributes
/// background jobs, and tags event envelopes. Callers that run the harness
/// (e.g. `--stage-run`) MUST pass the same id they later give the
/// orchestrator via `set_chat_session_id`, because the gate/refiner read the
/// ledger with `WHERE session_id = <chat_session_id>` — a mismatch makes every
/// booked evidence id invisible to them. The interactive CLI (REPL/runner)
/// passes `"cli"` (single-session mode, no session-scoped gate queries).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn initialize_agent(
    workspace: &Path,
    settings: &GolishSettings,
    args: &Args,
    runtime: Arc<dyn GolishRuntime>,
    indexer_state: Arc<IndexerState>,
    sidecar_state: Arc<SidecarState>,
    event_session_id: &str,
    knowledge_memory: Option<Arc<dyn golish_memory_app::KnowledgeUnitOfWork>>,
    knowledge_context: Option<Arc<dyn golish_memory_app::ContextPackProvider>>,
) -> Result<(AgentBridge, Option<Arc<golish_mcp::McpManager>>)> {
    // Resolve and validate the same typed provider configuration consumed by
    // the GUI. Unknown provider names fail here instead of silently becoming
    // an OpenRouter client with the wrong endpoint.
    let bootstrap = resolve_cli_agent_bootstrap(workspace, settings, args)?;
    let provider_config = bootstrap.provider_config;
    let shared_config = bootstrap.shared_components_config;
    let provider = provider_config.provider_name().to_string();
    let model = provider_config.model().to_string();

    if args.verbose {
        eprintln!("[cli] Provider: {}", provider);
        eprintln!("[cli] Model: {}", model);
    }

    let mut bridge = AgentBridge::from_provider_config(
        provider_config,
        shared_config,
        runtime,
        event_session_id,
    )
    .await?;

    // Inject dependencies (same as init_ai_agent command in Tauri)
    if let Some(knowledge_memory) = knowledge_memory {
        bridge.set_knowledge_memory(knowledge_memory);
    }
    if let Some(knowledge_context) = knowledge_context {
        bridge.set_knowledge_context(knowledge_context);
    }
    bridge.set_indexer_state(indexer_state);
    let sidecar_backend: std::sync::Arc<
        dyn golish_agent_kit::sidecar_trait::SessionCaptureBackend,
    > = std::sync::Arc::new(crate::ai::sidecar_bridge::SidecarCaptureBackend::new(
        sidecar_state,
    ));
    bridge.set_sidecar_state(sidecar_backend);

    // Initialize MCP (Model Context Protocol) integration
    // Load config from user-global (~/.golish/mcp.json) and project-specific paths
    // Auto-connect to enabled servers and expose tools to the agent
    let mcp_manager = if args.stage_run_test_investigation_llm_endpoint.is_some() {
        tracing::info!(
            target: "harness::stage_run",
            "deterministic Investigation fixture disabled MCP initialization"
        );
        None
    } else {
        match initialize_mcp_integration(&mut bridge, workspace, args.verbose).await {
            Ok(manager) => manager,
            Err(e) => {
                if args.verbose {
                    eprintln!("[cli] Warning: Failed to initialize MCP: {}", e);
                }
                tracing::warn!("[mcp] Failed to initialize MCP integration: {}", e);
                // Non-fatal: agent continues without MCP tools
                None
            }
        }
    };

    Ok((bridge, mcp_manager))
}

/// Convert CLI/settings input into the exact typed provider configuration used
/// by the GUI session initializer. This is intentionally pure apart from API-key
/// environment lookup so provider routing can be covered without constructing a
/// bridge or starting a runtime operation.
fn resolve_cli_provider_config(
    workspace: &Path,
    settings: &GolishSettings,
    args: &Args,
) -> Result<ProviderConfig> {
    use crate::settings::schema::AiProvider;

    let requested = args
        .provider
        .clone()
        .unwrap_or_else(|| settings.ai.default_provider.to_string());
    let provider = requested
        .parse::<AiProvider>()
        .map_err(|_| anyhow::anyhow!("Unsupported CLI AI provider '{requested}'"))?;
    let provider_name = provider.to_string();
    let model = args
        .model
        .clone()
        .unwrap_or_else(|| settings.ai.default_model.clone());
    let workspace = workspace.to_string_lossy().into_owned();
    let config = match provider {
        AiProvider::VertexAi => ProviderConfig::VertexAi {
            workspace,
            model,
            credentials_path: settings.ai.vertex_ai.credentials_path.clone(),
            project_id: settings.ai.vertex_ai.project_id.clone().ok_or_else(|| {
                anyhow::anyhow!("Vertex AI requires 'ai.vertex_ai.project_id' in settings.toml")
            })?,
            location: String::new(),
            model_override: None,
        },
        AiProvider::VertexGemini => ProviderConfig::VertexGemini {
            workspace,
            model,
            credentials_path: settings.ai.vertex_gemini.credentials_path.clone(),
            project_id: settings
                .ai
                .vertex_gemini
                .project_id
                .clone()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Vertex Gemini requires 'ai.vertex_gemini.project_id' in settings.toml"
                    )
                })?,
            location: String::new(),
            include_thoughts: false,
            model_override: None,
        },
        AiProvider::Openrouter => ProviderConfig::Openrouter {
            workspace,
            model,
            api_key: resolve_api_key(settings, &provider_name, args)?,
            provider_preferences: None,
            model_override: None,
        },
        AiProvider::Openai => ProviderConfig::Openai {
            workspace,
            model,
            api_key: resolve_api_key(settings, &provider_name, args)?,
            base_url: None,
            reasoning_effort: None,
            enable_web_search: false,
            web_search_context_size: String::new(),
            model_override: None,
        },
        AiProvider::Anthropic => ProviderConfig::Anthropic {
            workspace,
            model,
            api_key: resolve_api_key(settings, &provider_name, args)?,
            model_override: None,
        },
        AiProvider::Ollama => ProviderConfig::Ollama {
            workspace,
            model,
            base_url: None,
            model_override: None,
        },
        AiProvider::Gemini => ProviderConfig::Gemini {
            workspace,
            model,
            api_key: resolve_api_key(settings, &provider_name, args)?,
            include_thoughts: false,
            model_override: None,
        },
        AiProvider::Groq => ProviderConfig::Groq {
            workspace,
            model,
            api_key: resolve_api_key(settings, &provider_name, args)?,
            model_override: None,
        },
        AiProvider::Xai => ProviderConfig::Xai {
            workspace,
            model,
            api_key: resolve_api_key(settings, &provider_name, args)?,
            model_override: None,
        },
        AiProvider::ZaiSdk => ProviderConfig::ZaiSdk {
            workspace,
            model,
            api_key: resolve_api_key(settings, &provider_name, args)?,
            base_url: None,
            source_channel: None,
            model_override: None,
        },
        AiProvider::Nvidia => ProviderConfig::Nvidia {
            workspace,
            model,
            api_key: resolve_api_key(settings, &provider_name, args)?,
            base_url: None,
            model_override: None,
        },
        AiProvider::Deepseek => ProviderConfig::Deepseek {
            workspace,
            model,
            api_key: resolve_api_key(settings, &provider_name, args)?,
            base_url: None,
            model_override: None,
        },
        AiProvider::Xiaomi => ProviderConfig::Xiaomi {
            workspace,
            model,
            api_key: resolve_api_key(settings, &provider_name, args)?,
            region: None,
            default_protocol: None,
            base_url: None,
            anthropic_base_url: None,
            model_override: None,
        },
    };
    Ok(config)
}

fn resolve_cli_agent_bootstrap(
    workspace: &Path,
    settings: &GolishSettings,
    args: &Args,
) -> Result<AgentBootstrapConfig> {
    Ok(normalize_agent_bootstrap(
        resolve_cli_provider_config(workspace, settings, args)?,
        settings,
    ))
}

/// Initialize MCP integration for the agent bridge.
/// Loads config, connects to enabled servers, and sets up tool definitions + executor.
/// Returns the MCP manager so it can be stored for shutdown.
#[allow(dead_code)]
pub(super) async fn initialize_mcp_integration(
    bridge: &mut AgentBridge,
    workspace: &Path,
    verbose: bool,
) -> Result<Option<Arc<golish_mcp::McpManager>>> {
    use golish_mcp::{load_mcp_config, McpManager};

    // Load MCP config (merges user-global and project-specific)
    let config = load_mcp_config(workspace)?;

    if config.mcp_servers.is_empty() {
        tracing::debug!("[mcp] No MCP servers configured");
        return Ok(None);
    }

    if verbose {
        eprintln!(
            "[cli] Found {} MCP servers in config",
            config.mcp_servers.len()
        );
    }

    // Create manager and connect to all enabled servers
    let manager = Arc::new(McpManager::new(config.mcp_servers));
    if let Err(e) = manager.connect_all().await {
        tracing::warn!("[mcp] Failed to connect to some MCP servers: {}", e);
        // Continue anyway - some servers may have connected
    }

    // Get all available tools from connected servers
    let tools = manager.list_tools().await?;
    let tool_definitions: Vec<rig::completion::ToolDefinition> =
        tools.iter().map(|tool| tool.to_tool_definition()).collect();

    if verbose {
        eprintln!(
            "[cli] Loaded {} tools from MCP servers",
            tool_definitions.len()
        );
    }
    tracing::info!(
        "[mcp] Loaded {} tools from MCP servers",
        tool_definitions.len()
    );

    let executor: Arc<dyn golish_agent_runtime::agentic_loop::McpToolExecutor> = Arc::new(
        crate::ai::commands::McpManagerToolExecutor::new(Arc::clone(&manager)),
    );

    bridge.set_mcp_tools(tool_definitions).await;
    bridge.set_mcp_executor(executor).await;

    Ok(Some(manager))
}

/// Resolve API key from CLI args, settings, or environment variables.
#[allow(dead_code)]
pub(super) fn resolve_api_key(
    settings: &GolishSettings,
    provider: &str,
    args: &Args,
) -> Result<String> {
    // 1. CLI argument takes precedence
    if let Some(ref key) = args.api_key {
        return Ok(key.clone());
    }

    // 2. Check settings based on provider
    let from_settings = match provider {
        "openrouter" => get_with_env_fallback(
            &settings.ai.openrouter.api_key,
            &["OPENROUTER_API_KEY"],
            None,
        ),
        "anthropic" => {
            get_with_env_fallback(&settings.ai.anthropic.api_key, &["ANTHROPIC_API_KEY"], None)
        }
        "openai" => get_with_env_fallback(&settings.ai.openai.api_key, &["OPENAI_API_KEY"], None),
        "gemini" => get_with_env_fallback(&settings.ai.gemini.api_key, &["GEMINI_API_KEY"], None),
        "groq" => get_with_env_fallback(&settings.ai.groq.api_key, &["GROQ_API_KEY"], None),
        "xai" => get_with_env_fallback(&settings.ai.xai.api_key, &["XAI_API_KEY"], None),
        "zai_sdk" => get_with_env_fallback(&settings.ai.zai_sdk.api_key, &["ZAI_API_KEY"], None),
        "nvidia" | "nvidia_nim" | "nim" => {
            get_with_env_fallback(&settings.ai.nvidia.api_key, &["NVIDIA_API_KEY"], None)
        }
        "xiaomi" => get_with_env_fallback(&settings.ai.xiaomi.api_key, &["XIAOMI_API_KEY"], None),
        "deepseek" | "deepseek_api" => {
            get_with_env_fallback(&settings.ai.deepseek.api_key, &["DEEPSEEK_API_KEY"], None)
        }
        _ => None,
    };

    from_settings.ok_or_else(|| {
        anyhow::anyhow!(
            "No API key found for provider '{}'. Set it in ~/.golish/settings.toml, \
             via environment variable, or use --api-key",
            provider
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_agent_kit::llm_client::ProviderConfig;

    #[test]
    fn test_resolve_api_key_from_args() {
        let settings = GolishSettings::default();
        let mut args = Args::parse_from(["golish-cli"]);
        args.api_key = Some("test-key".to_string());

        let key = resolve_api_key(&settings, "openrouter", &args).unwrap();
        assert_eq!(key, "test-key");
    }

    #[test]
    fn cli_provider_config_rejects_unknown_provider_instead_of_openrouter_fallback() {
        let settings = GolishSettings::default();
        let args = Args::parse_from([
            "golish-cli",
            "--provider",
            "typo-provider",
            "--api-key",
            "unused",
        ]);

        let error = match resolve_cli_agent_bootstrap(Path::new("/tmp/workspace"), &settings, &args)
        {
            Ok(_) => panic!("unknown provider must fail before bridge construction"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("Unsupported CLI AI provider"));
    }

    #[test]
    fn cli_provider_config_routes_openai_through_the_gui_typed_factory() {
        let settings = GolishSettings::default();
        let args = Args::parse_from([
            "golish-cli",
            "--provider",
            "openai",
            "--model",
            "gpt-test",
            "--api-key",
            "test-key",
        ]);

        let config = resolve_cli_agent_bootstrap(Path::new("/tmp/workspace"), &settings, &args)
            .expect("OpenAI CLI config")
            .provider_config;

        assert!(matches!(
            config,
            ProviderConfig::Openai { model, api_key, .. }
                if model == "gpt-test" && api_key == "test-key"
        ));
    }

    #[test]
    fn cli_provider_config_routes_anthropic_through_the_gui_typed_factory() {
        let settings = GolishSettings::default();
        let args = Args::parse_from([
            "golish-cli",
            "--provider",
            "anthropic",
            "--model",
            "claude-test",
            "--api-key",
            "test-key",
        ]);

        let config = resolve_cli_agent_bootstrap(Path::new("/tmp/workspace"), &settings, &args)
            .expect("Anthropic CLI config")
            .provider_config;

        assert!(matches!(
            config,
            ProviderConfig::Anthropic { model, api_key, .. }
                if model == "claude-test" && api_key == "test-key"
        ));
    }

    #[test]
    fn cli_bootstrap_keeps_flag_overrides_and_adds_shared_settings() {
        let mut settings = GolishSettings::default();
        settings.ai.openai.base_url = Some("https://settings.openai.test/v1".to_string());
        settings.ai.openai.enable_web_search = true;
        settings.context.enabled = true;
        settings.context.compaction_threshold = 0.81;
        let args = Args::parse_from([
            "golish-cli",
            "--provider",
            "openai",
            "--model",
            "gpt-cli-override",
            "--api-key",
            "cli-key-override",
        ]);

        let bootstrap = resolve_cli_agent_bootstrap(Path::new("/tmp/workspace"), &settings, &args)
            .expect("normalized CLI bootstrap");

        assert!(matches!(
            bootstrap.provider_config,
            ProviderConfig::Openai {
                model,
                api_key,
                base_url: Some(base_url),
                enable_web_search: true,
                ..
            } if model == "gpt-cli-override"
                && api_key == "cli-key-override"
                && base_url == "https://settings.openai.test/v1"
        ));
        let context = bootstrap
            .shared_components_config
            .context_config
            .expect("CLI must share GUI context settings");
        assert_eq!(context.compaction_threshold, 0.81);
    }

    // Helper to create Args for testing
    use clap::Parser;
}
