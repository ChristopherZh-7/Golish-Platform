//! Agent initialization and API key resolution helpers.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::ai::agent_bridge::AgentBridge;
use crate::indexer::IndexerState;
use crate::settings::{get_with_env_fallback, GolishSettings};
use crate::sidecar::SidecarState;
use golish_agent_kit::llm_client::SharedComponentsConfig;
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
) -> Result<(AgentBridge, Option<Arc<golish_mcp::McpManager>>)> {
    // Resolve provider: CLI arg > settings > default
    let provider = args
        .provider
        .clone()
        .unwrap_or_else(|| settings.ai.default_provider.to_string());

    // Resolve model: CLI arg > settings > provider-specific default
    let model = args
        .model
        .clone()
        .unwrap_or_else(|| settings.ai.default_model.clone());

    if args.verbose {
        eprintln!("[cli] Provider: {}", provider);
        eprintln!("[cli] Model: {}", model);
    }

    // Create shared config with settings
    let shared_config = SharedComponentsConfig {
        settings: settings.clone(),
        context_config: None,
    };

    // Create the agent bridge based on provider
    let mut bridge = match provider.as_str() {
        "vertex_ai" | "vertex" => {
            let creds_path = settings.ai.vertex_ai.credentials_path.clone();

            let project_id = settings.ai.vertex_ai.project_id.clone().ok_or_else(|| {
                anyhow::anyhow!("Vertex AI requires 'ai.vertex_ai.project_id' in settings.toml")
            })?;

            let location = settings
                .ai
                .vertex_ai
                .location
                .clone()
                .unwrap_or_else(|| "us-east5".to_string());

            if args.verbose {
                match &creds_path {
                    Some(p) => eprintln!("[cli] Vertex AI credentials: {}", p),
                    None => eprintln!("[cli] Vertex AI credentials: application default"),
                }
                eprintln!("[cli] Vertex AI project: {}", project_id);
                eprintln!("[cli] Vertex AI location: {}", location);
            }

            AgentBridge::new_vertex_anthropic_with_shared_config(
                workspace.to_path_buf(),
                creds_path.as_deref(),
                &project_id,
                &location,
                &model,
                shared_config,
                runtime,
                event_session_id,
            )
            .await?
        }
        "zai_sdk" => {
            let api_key = resolve_api_key(settings, &provider, args)?;
            let base_url = settings.ai.zai_sdk.base_url.clone();

            if args.verbose {
                if let Some(ref url) = base_url {
                    eprintln!("[cli] Z.AI SDK base URL: {}", url);
                } else {
                    eprintln!("[cli] Z.AI SDK base URL: default");
                }
            }

            AgentBridge::new_zai_sdk_with_shared_config(
                workspace.to_path_buf(),
                &model,
                &api_key,
                base_url.as_deref(),
                None, // source_channel
                shared_config,
                runtime,
                event_session_id,
            )
            .await?
        }
        "nvidia" | "nvidia_nim" | "nim" => {
            let api_key = resolve_api_key(settings, "nvidia", args)?;
            let base_url = settings.ai.nvidia.base_url.clone();

            if args.verbose {
                if let Some(ref url) = base_url {
                    eprintln!("[cli] NVIDIA NIM base URL: {}", url);
                } else {
                    eprintln!("[cli] NVIDIA NIM base URL: default");
                }
            }

            AgentBridge::new_nvidia_with_shared_config(
                workspace.to_path_buf(),
                &model,
                &api_key,
                base_url.as_deref(),
                shared_config,
                runtime,
                event_session_id,
            )
            .await?
        }
        "openrouter" => {
            let api_key = resolve_api_key(settings, &provider, args)?;

            // Build provider preferences JSON from settings (if configured)
            let provider_preferences = settings
                .ai
                .openrouter
                .provider_preferences
                .as_ref()
                .filter(|p| !p.is_empty())
                .map(golish_llm_providers::openrouter_preferences_to_json);

            if args.verbose && provider_preferences.is_some() {
                eprintln!("[cli] OpenRouter provider preferences configured");
            }

            AgentBridge::new_openrouter_with_shared_config(
                workspace.to_path_buf(),
                &model,
                &api_key,
                provider_preferences,
                shared_config,
                runtime,
                event_session_id,
            )
            .await?
        }
        "deepseek" | "deepseek_api" => {
            let api_key = resolve_api_key(settings, "deepseek", args)?;
            let base_url = settings.ai.deepseek.base_url.clone();

            if args.verbose {
                match base_url.as_deref() {
                    Some(url) => eprintln!("[cli] DeepSeek base URL: {}", url),
                    None => eprintln!("[cli] DeepSeek base URL: default (api.deepseek.com)"),
                }
            }

            AgentBridge::new_deepseek_with_shared_config(
                workspace.to_path_buf(),
                &model,
                &api_key,
                base_url.as_deref(),
                shared_config,
                runtime,
                event_session_id,
            )
            .await?
        }
        "xiaomi" => {
            let api_key = resolve_api_key(settings, "xiaomi", args)?;

            if args.verbose {
                match settings.ai.xiaomi.openai_base_url.as_deref() {
                    Some(url) => eprintln!("[cli] Xiaomi base URL: {}", url),
                    None => eprintln!("[cli] Xiaomi base URL: region-derived"),
                }
            }

            AgentBridge::new_xiaomi_with_shared_config(
                workspace.to_path_buf(),
                &model,
                &api_key,
                settings.ai.xiaomi.region.as_deref(),
                settings.ai.xiaomi.default_protocol.as_deref(),
                settings.ai.xiaomi.openai_base_url.as_deref(),
                settings.ai.xiaomi.anthropic_base_url.as_deref(),
                shared_config,
                runtime,
                event_session_id,
            )
            .await?
        }
        _ => {
            // API key-based providers (anthropic, openai, etc.)
            //
            // NOTE: this fallback always builds an OpenRouter client regardless
            // of `provider`. Providers that need their own endpoint (xiaomi,
            // nvidia, zai_sdk, vertex...) must have an explicit arm above, or
            // they will be silently misrouted to OpenRouter (→ 401/wrong host).
            let api_key = resolve_api_key(settings, &provider, args)?;

            AgentBridge::new_openrouter_with_shared_config(
                workspace.to_path_buf(),
                &model,
                &api_key,
                None,
                shared_config,
                runtime,
                event_session_id,
            )
            .await?
        }
    };

    // Inject dependencies (same as init_ai_agent command in Tauri)
    if let Some(knowledge_memory) = knowledge_memory {
        bridge.set_knowledge_memory(knowledge_memory);
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
    let mcp_manager = match initialize_mcp_integration(&mut bridge, workspace, args.verbose).await {
        Ok(manager) => manager,
        Err(e) => {
            if args.verbose {
                eprintln!("[cli] Warning: Failed to initialize MCP: {}", e);
            }
            tracing::warn!("[mcp] Failed to initialize MCP integration: {}", e);
            // Non-fatal: agent continues without MCP tools
            None
        }
    };

    Ok((bridge, mcp_manager))
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

    #[test]
    fn test_resolve_api_key_from_args() {
        let settings = GolishSettings::default();
        let mut args = Args::parse_from(["golish-cli"]);
        args.api_key = Some("test-key".to_string());

        let key = resolve_api_key(&settings, "openrouter", &args).unwrap();
        assert_eq!(key, "test-key");
    }

    // Helper to create Args for testing
    use clap::Parser;
}
