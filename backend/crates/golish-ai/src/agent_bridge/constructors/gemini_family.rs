//! Gemini providers: native Gemini API and Vertex Gemini.
//!
//! Each provider exposes three layered constructors:
//! - `new_*_with_runtime`: minimal config, defaults applied.
//! - `new_*_with_context`: adds an optional `ContextManagerConfig`.
//! - `new_*_with_shared_config`: full control via `SharedComponentsConfig`.
//! All three funnel into [`AgentBridge::from_components_with_runtime`].

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use golish_context::ContextManagerConfig;
use golish_core::runtime::GolishRuntime;

use crate::llm_client::{
    create_gemini_components, create_vertex_gemini_components, GeminiClientConfig,
    SharedComponentsConfig, VertexGeminiClientConfig,
};

use super::super::AgentBridge;

impl AgentBridge {

    pub async fn new_gemini_with_runtime(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_gemini_with_context(workspace, model, api_key, None, runtime).await
    }


    pub async fn new_gemini_with_context(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_gemini_with_shared_config(workspace, model, api_key, shared_config, runtime, "")
            .await
    }


    pub async fn new_gemini_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = GeminiClientConfig {
            workspace,
            model,
            api_key,
        };
        let components = create_gemini_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }


    pub async fn new_vertex_gemini_with_runtime(
        workspace: PathBuf,
        credentials_path: Option<&str>,
        project_id: &str,
        location: &str,
        model: &str,
        include_thoughts: bool,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_vertex_gemini_with_context(
            workspace,
            credentials_path,
            project_id,
            location,
            model,
            include_thoughts,
            None,
            runtime,
        )
        .await
    }


    #[allow(clippy::too_many_arguments)]
    pub async fn new_vertex_gemini_with_context(
        workspace: PathBuf,
        credentials_path: Option<&str>,
        project_id: &str,
        location: &str,
        model: &str,
        include_thoughts: bool,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_vertex_gemini_with_shared_config(
            workspace,
            credentials_path,
            project_id,
            location,
            model,
            include_thoughts,
            shared_config,
            runtime,
            "",
        )
        .await
    }


    #[allow(clippy::too_many_arguments)]
    pub async fn new_vertex_gemini_with_shared_config(
        workspace: PathBuf,
        credentials_path: Option<&str>,
        project_id: &str,
        location: &str,
        model: &str,
        include_thoughts: bool,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = VertexGeminiClientConfig {
            workspace,
            credentials_path,
            project_id,
            location,
            model,
            include_thoughts,
        };

        let components = create_vertex_gemini_components(config, shared_config).await?;

        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

}
