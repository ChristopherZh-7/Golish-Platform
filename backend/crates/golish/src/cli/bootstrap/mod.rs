//! CLI bootstrap - Initialize the full Golish stack for CLI usage.
//!
//! This module provides `CliContext` which initializes all the same services
//! as the Tauri GUI application, ensuring feature parity between CLI and GUI.

mod agent_init;
use agent_init::initialize_agent;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{mpsc, RwLock};

use crate::ai::agent_bridge::AgentBridge;
use crate::history::{HistoryConfig, HistoryManager};
use crate::indexer::IndexerState;
use crate::pty::PtyManager;
use crate::runtime::CliRuntime;
use crate::settings::SettingsManager;
use crate::sidecar::SidecarState;
use golish_core::runtime::{GolishRuntime, RuntimeEvent};

use super::args::Args;

/// Context for CLI execution containing all initialized services.
///
/// This mirrors the Tauri `AppState` but is owned rather than managed by Tauri.
pub struct CliContext {
    /// Runtime abstraction for event emission
    pub runtime: Arc<dyn GolishRuntime>,

    /// Global history manager (best-effort)
    pub history: Option<crate::history::HistoryManager>,

    /// Resolved provider/model used by the CLI for this run (for history metadata)
    pub provider: String,
    pub model: String,

    /// Event receiver for output handling
    pub event_rx: mpsc::UnboundedReceiver<RuntimeEvent>,

    /// Agent bridge (initialized lazily via `ensure_agent`)
    bridge: Arc<RwLock<Option<AgentBridge>>>,

    /// Resolved workspace path
    pub workspace: PathBuf,

    /// Settings manager
    pub settings_manager: Arc<SettingsManager>,

    /// PTY manager for shell execution
    pub pty_manager: Arc<PtyManager>,

    /// Code indexer
    pub indexer_state: Arc<IndexerState>,

    /// Sidecar context capture
    pub sidecar_state: Arc<SidecarState>,

    /// MCP manager for external tool servers (optional)
    pub mcp_manager: Option<Arc<golish_mcp::McpManager>>,

    /// Command-line arguments
    pub args: Args,
}

impl CliContext {
    /// Get a reference to the agent bridge, if initialized.
    pub async fn bridge(&self) -> tokio::sync::RwLockReadGuard<'_, Option<AgentBridge>> {
        self.bridge.read().await
    }

    /// Get a mutable reference to the agent bridge.
    pub async fn bridge_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, Option<AgentBridge>> {
        self.bridge.write().await
    }

    /// Check if the agent is initialized.
    pub async fn is_agent_initialized(&self) -> bool {
        self.bridge.read().await.is_some()
    }

    /// Graceful shutdown - flush sidecar, end sessions, MCP servers, etc.
    pub async fn shutdown(self) -> Result<()> {
        // Finalize agent session if needed
        if let Some(ref bridge) = *self.bridge.read().await {
            bridge.finalize_session().await;
        }

        // Shutdown MCP servers (cancels child processes gracefully)
        if let Some(ref manager) = self.mcp_manager {
            manager.shutdown().await;
        }

        // Gracefully shutdown sidecar (waits for processor to flush pending events)
        self.sidecar_state.shutdown();

        // Shutdown the runtime
        if let Err(e) = self.runtime.shutdown().await {
            tracing::warn!("Runtime shutdown error: {}", e);
        }

        Ok(())
    }
}

/// Initialize the CLI context with all services.
///
/// This is the main entry point for CLI initialization, mirroring
/// what happens in the Tauri app's `AppState::new()` and `init_ai_agent`.
pub async fn initialize(args: &Args) -> Result<CliContext> {
    // Install TLS provider (required for rustls 0.23+)
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Load .env file if present
    if let Err(e) = dotenvy::dotenv() {
        // Only warn on errors other than file not found
        if !matches!(e, dotenvy::Error::Io(_)) {
            tracing::warn!("Failed to load .env file: {}", e);
        }
    }

    // Set session directory to ~/.golish/sessions
    if std::env::var_os("VT_SESSION_DIR").is_none() {
        if let Some(home) = dirs::home_dir() {
            let golish_sessions = home.join(".golish").join("sessions");
            std::env::set_var("VT_SESSION_DIR", &golish_sessions);
        }
    }

    // Determine log level based on verbosity
    let log_level = if args.verbose { "debug" } else { "warn" };

    // Resolve workspace path
    let workspace = args.resolve_workspace()?;

    if args.verbose {
        eprintln!("[cli] Workspace: {}", workspace.display());
    }

    // Load settings
    let settings_manager = Arc::new(
        SettingsManager::new()
            .await
            .context("Failed to initialize settings manager")?,
    );

    // Ensure settings file exists (creates template on first run)
    if let Err(e) = settings_manager.ensure_settings_file().await {
        // Can't use tracing yet, use eprintln
        eprintln!("[cli] Warning: Failed to create settings template: {}", e);
    }

    let settings = settings_manager.get().await;

    // Apply proxy settings as environment variables
    golish_settings::apply_proxy_env(&settings);

    // Initialize tracing with optional Langfuse export
    let langfuse_config =
        crate::telemetry::LangfuseConfig::from_settings(&settings.telemetry.langfuse);

    // Build log directives based on mode
    #[allow(unused_mut)] // mutated when evals feature is enabled
    let mut directives: Vec<String> = vec![
        format!("golish={}", log_level),
        format!("golish_evals={}", log_level),
        format!("golish_ai={}", log_level),
    ];

    // In eval mode, suppress noisy internal logs to keep output clean
    #[cfg(feature = "evals")]
    if args.eval {
        // Suppress agentic loop details (compaction checks, iteration logs)
        directives.push("golish_ai::agentic_loop=warn".to_string());
        // Suppress system hooks debug logs
        directives.push("golish_ai::system_hooks=warn".to_string());
        // Suppress sub-agent executor details
        directives.push("golish_sub_agents::executor=warn".to_string());
    }

    let extra_directives: Vec<&str> = directives.iter().map(|s| s.as_str()).collect();

    // Initialize telemetry (this sets up the global subscriber)
    // We ignore the guard since CLI runs to completion
    if let Err(e) = crate::telemetry::init_tracing(langfuse_config, log_level, &extra_directives) {
        eprintln!("[cli] Warning: Failed to initialize tracing: {}", e);
        // Fall back to basic tracing
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive(format!("golish={}", log_level).parse().unwrap()),
            )
            .try_init();
    }

    if args.verbose {
        eprintln!(
            "[cli] Settings loaded from {}",
            settings_manager.path().display()
        );
        eprintln!("[cli] Default provider: {}", settings.ai.default_provider);
        eprintln!("[cli] Default model: {}", settings.ai.default_model);
        if settings.telemetry.langfuse.enabled {
            eprintln!("[cli] Langfuse tracing enabled");
        }
    }

    // Create event channel
    let (event_tx, event_rx) = mpsc::unbounded_channel::<RuntimeEvent>();

    // Create CLI runtime
    let runtime: Arc<dyn GolishRuntime> =
        Arc::new(CliRuntime::new(event_tx, args.auto_approve, args.json));

    // Initialize services
    let pty_manager = Arc::new(PtyManager::new());
    let indexer_state = Arc::new(IndexerState::new());
    let sidecar_state = Arc::new(SidecarState::new());
    let history = HistoryManager::new(HistoryConfig::default()).ok();

    // Initialize sidecar
    if settings.sidecar.enabled {
        if let Err(e) = sidecar_state.initialize(workspace.clone()).await {
            tracing::warn!("Failed to initialize sidecar: {}", e);
        } else if args.verbose {
            eprintln!("[cli] Sidecar initialized");
        }
    }

    // Resolve provider/model for this run (used for history metadata)
    let provider = args
        .provider
        .clone()
        .unwrap_or_else(|| settings.ai.default_provider.to_string());
    let model = args
        .model
        .clone()
        .unwrap_or_else(|| settings.ai.default_model.clone());

    // Initialize the agent bridge and MCP manager
    let (bridge, mcp_manager) = initialize_agent(
        &workspace,
        &settings,
        args,
        runtime.clone(),
        indexer_state.clone(),
        sidecar_state.clone(),
    )
    .await?;

    if args.verbose {
        eprintln!("[cli] Agent initialized successfully");
    }

    Ok(CliContext {
        runtime,
        history,
        provider,
        model,
        event_rx,
        bridge: Arc::new(RwLock::new(Some(bridge))),
        workspace,
        settings_manager,
        pty_manager,
        indexer_state,
        sidecar_state,
        mcp_manager,
        args: args.clone(),
    })
}

