//! Lifecycle / status / configuration methods on [`SidecarState`].

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;

use anyhow::Result;
use tauri::AppHandle;

use super::super::config::SidecarConfig;
use super::super::events::SidecarEvent;
use super::super::processor::{Processor, ProcessorConfig};
use super::super::session::ensure_sessions_dir;

use super::SidecarState;
use super::SidecarStatus;
use super::InternalState;

impl SidecarState {
    /// Create a new SidecarState with default configuration
    pub fn new() -> Self {
        Self {
            config: RwLock::new(SidecarConfig::default()),
            state: RwLock::new(InternalState::default()),
            processor: RwLock::new(None),
            app_handle: RwLock::new(None),
        }
    }

    /// Create a new SidecarState with custom configuration
    pub fn with_config(config: SidecarConfig) -> Self {
        Self {
            config: RwLock::new(config),
            state: RwLock::new(InternalState::default()),
            processor: RwLock::new(None),
            app_handle: RwLock::new(None),
        }
    }

    /// Set the Tauri app handle for event emission
    pub fn set_app_handle(&self, handle: AppHandle) {
        *self.app_handle.write().unwrap() = Some(handle);
    }

    /// Emit a sidecar event to the frontend
    pub fn emit_event(&self, event: SidecarEvent) {
        use tauri::Emitter;
        if let Some(handle) = self.app_handle.read().unwrap().as_ref() {
            if let Err(e) = handle.emit("sidecar-event", &event) {
                tracing::warn!("Failed to emit sidecar event: {}", e);
            }
        }
    }

    /// Initialize the sidecar system
    pub async fn initialize(&self, workspace: PathBuf) -> Result<()> {
        let config = self.config.read().unwrap().clone();

        if !config.enabled {
            tracing::trace!("Sidecar is disabled, skipping initialization");
            return Ok(());
        }

        // Ensure sessions directory exists
        let sessions_dir = config.sessions_dir();
        ensure_sessions_dir(&sessions_dir).await?;

        // Create processor with synthesis config from sidecar config
        let synthesis_config = golish_synthesis::SynthesisConfig {
            enabled: config.synthesis_enabled,
            backend: config.synthesis_backend,
            vertex: config.synthesis_vertex.clone(),
            openai: config.synthesis_openai.clone(),
            grok: config.synthesis_grok.clone(),
        };

        tracing::info!(
            "[sidecar-state] Creating synthesis config: enabled={}, backend={:?}",
            synthesis_config.enabled,
            synthesis_config.backend
        );

        // Get app handle for processor to emit events
        let app_handle_arc = self
            .app_handle
            .read()
            .unwrap()
            .as_ref()
            .map(|h| Arc::new(h.clone()));

        let processor_config = ProcessorConfig {
            sessions_dir: sessions_dir.clone(),
            generate_patches: true,
            synthesis: synthesis_config,
            app_handle: app_handle_arc,
        };
        let processor = Processor::spawn(processor_config);

        // Update state
        {
            let mut state = self.state.write().unwrap();
            state.workspace_path = Some(workspace);
            state.initialized = true;
        }
        {
            *self.processor.write().unwrap() = Some(processor);
        }

        tracing::info!("Sidecar initialized with sessions dir: {:?}", sessions_dir);
        Ok(())
    }

    /// Get current status
    pub fn status(&self) -> SidecarStatus {
        let config = self.config.read().unwrap();
        let state = self.state.read().unwrap();

        SidecarStatus {
            active_session: state.current_session_id.is_some(),
            session_id: state.current_session_id.clone(),
            enabled: config.enabled,
            sessions_dir: config.sessions_dir(),
            workspace_path: state.workspace_path.clone(),
        }
    }

    /// Get current configuration
    pub fn config(&self) -> SidecarConfig {
        self.config.read().unwrap().clone()
    }

    /// Update configuration
    pub fn set_config(&self, config: SidecarConfig) {
        *self.config.write().unwrap() = config;
    }

    /// Shutdown the sidecar
    ///
    /// This waits for the processor to finish any pending work (like patch generation)
    /// before returning. The processor handles EndSession which generates patches.
    pub fn shutdown(&self) {
        let _ = self.end_session();

        if let Some(processor) = self.processor.write().unwrap().take() {
            // Spawn a thread with its own runtime to shutdown the processor.
            // The processor.shutdown() now awaits the task handle, ensuring all
            // pending work (including patch generation) completes before returning.
            let handle = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(processor.shutdown());
            });

            // Wait for the processor to finish all pending work
            if let Err(e) = handle.join() {
                tracing::warn!("Processor shutdown thread panicked: {:?}", e);
            }
        }

        tracing::info!("Sidecar shutdown complete");
    }
}
