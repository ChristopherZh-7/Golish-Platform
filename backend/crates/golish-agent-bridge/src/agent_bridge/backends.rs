//! `BridgeBackends` — host-supplied trait-object backends bundled into a
//! single struct so `AgentBridge::apply_backends()` can wire them up in
//! one call.
//!
//! Replaces the 8-call `set_*` recipe that hosts (e.g. the `golish` Tauri
//! application's `configure_bridge`) used to follow when constructing a
//! per-session bridge. New code should build a `BridgeBackends` literal
//! and call [`AgentBridge::apply_backends`] instead of poking individual
//! setters one-by-one.
//!
//! Two setters intentionally stay out of `BridgeBackends`:
//! - [`AgentBridge::set_db_backend`] — its readiness-gate parameter
//!   uses a `Clone + 'static` generic bound that doesn't trivially
//!   survive a `Box<dyn>` round-trip, so callers keep invoking it
//!   directly.
//! - [`AgentBridge::set_memory_file_path`] — async, needs `await`.

use std::sync::Arc;

use golish_indexer::IndexerState;

use crate::agentic_loop::{OutputClassifier, PostShellHook};
use crate::db_traits::{DbRepoProvider, TextEmbedder};
use crate::llm_client::LlmClientFactory;
use crate::sidecar_trait::SessionCaptureBackend;
use crate::tool_executors::graph_trait::GraphKnowledgeBase;

use super::AgentBridge;

/// Aggregate of host-provided trait-object backends to inject into a
/// freshly built [`AgentBridge`].
///
/// Build with default + chained setters (or struct-update syntax) and
/// hand off to [`AgentBridge::apply_backends`]. Every field is optional;
/// `None` means "do not touch the corresponding bridge service".
#[derive(Default)]
pub struct BridgeBackends {
    /// File-tree indexer state (powers ast-grep / file-search tools).
    pub indexer: Option<Arc<IndexerState>>,
    /// Sidecar capture backend (artifact + patch staging).
    pub sidecar: Option<Arc<dyn SessionCaptureBackend>>,
    /// App-wide settings manager (consumed by `LlmClientFactory` and
    /// the dynamic memory-file lookup).
    pub settings: Option<Arc<golish_settings::SettingsManager>>,
    /// Domain repository provider for tool executors.
    /// Forwarded into the live `db_tracker`; only takes effect if
    /// `set_db_backend` has already been called.
    pub db_repo: Option<Arc<dyn DbRepoProvider>>,
    /// Graphiti / knowledge-graph backend.
    pub graph: Option<Arc<dyn GraphKnowledgeBase>>,
    /// Text-embedding backend (semantic memory).
    /// Forwarded into the live `db_tracker`; only takes effect if
    /// `set_db_backend` has already been called.
    pub embedder: Option<Arc<dyn TextEmbedder>>,
    /// LLM client factory used for sub-agent provider switching.
    pub model_factory: Option<Arc<LlmClientFactory>>,
    /// Hook fired after every shell-tool invocation completes.
    pub post_shell_hook: Option<PostShellHook>,
    /// Classifier deciding whether shell output is forwarded to
    /// structured-storage capture.
    pub output_classifier: Option<OutputClassifier>,
}

impl AgentBridge {
    /// Apply a [`BridgeBackends`] bundle in one shot.
    ///
    /// Each `Some(...)` field invokes the matching `set_*` setter;
    /// `None` fields are skipped. Order matches existing manual setup
    /// in the `golish` host so that backends with cross-dependencies
    /// (e.g. `db_repo` and `embedder`, which both forward into the
    /// live db tracker) are applied after their prerequisites.
    pub fn apply_backends(&mut self, b: BridgeBackends) {
        if let Some(x) = b.indexer {
            self.set_indexer_state(x);
        }
        if let Some(x) = b.sidecar {
            self.set_sidecar_state(x);
        }
        if let Some(x) = b.settings {
            self.set_settings_manager(x);
        }
        if let Some(x) = b.graph {
            self.set_graph_backend(x);
        }
        // db_repo / embedder forward into the live tracker — caller
        // must have invoked `set_db_backend` first for them to stick.
        if let Some(x) = b.db_repo {
            self.set_db_repo(x);
        }
        if let Some(x) = b.embedder {
            self.set_embedder(x);
        }
        if let Some(x) = b.model_factory {
            self.set_model_factory(x);
        }
        if let Some(x) = b.post_shell_hook {
            self.set_post_shell_hook(x);
        }
        if let Some(x) = b.output_classifier {
            self.set_output_classifier(x);
        }
    }
}
