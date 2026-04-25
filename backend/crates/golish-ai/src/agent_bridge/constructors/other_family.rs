//! Misc providers: Ollama, Groq, xAI, Z.AI SDK.
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
    create_groq_components, create_ollama_components, create_xai_components,
    create_zai_sdk_components, GroqClientConfig, OllamaClientConfig, SharedComponentsConfig,
    XaiClientConfig, ZaiSdkClientConfig,
};

use super::super::AgentBridge;

impl AgentBridge {

    pub async fn new_ollama_with_runtime(
        workspace: PathBuf,
        model: &str,
        base_url: Option<&str>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_ollama_with_context(workspace, model, base_url, None, runtime).await
    }


    pub async fn new_ollama_with_context(
        workspace: PathBuf,
        model: &str,
        base_url: Option<&str>,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_ollama_with_shared_config(workspace, model, base_url, shared_config, runtime, "")
            .await
    }


    pub async fn new_ollama_with_shared_config(
        workspace: PathBuf,
        model: &str,
        base_url: Option<&str>,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = OllamaClientConfig {
            workspace,
            model,
            base_url,
        };
        let components = create_ollama_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }


    pub async fn new_groq_with_runtime(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_groq_with_context(workspace, model, api_key, None, runtime).await
    }


    pub async fn new_groq_with_context(
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
        Self::new_groq_with_shared_config(workspace, model, api_key, shared_config, runtime, "")
            .await
    }


    pub async fn new_groq_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = GroqClientConfig {
            workspace,
            model,
            api_key,
        };
        let components = create_groq_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }


    pub async fn new_xai_with_runtime(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_xai_with_context(workspace, model, api_key, None, runtime).await
    }


    pub async fn new_xai_with_context(
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
        Self::new_xai_with_shared_config(workspace, model, api_key, shared_config, runtime, "")
            .await
    }


    pub async fn new_xai_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = XaiClientConfig {
            workspace,
            model,
            api_key,
        };
        let components = create_xai_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }


    pub async fn new_zai_sdk_with_runtime(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        source_channel: Option<&str>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_zai_sdk_with_context(workspace, model, api_key, base_url, source_channel, None, runtime).await
    }


    #[allow(clippy::too_many_arguments)]
    pub async fn new_zai_sdk_with_context(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        source_channel: Option<&str>,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_zai_sdk_with_shared_config(
            workspace,
            model,
            api_key,
            base_url,
            source_channel,
            shared_config,
            runtime,
            "",
        )
        .await
    }


    #[allow(clippy::too_many_arguments)]
    pub async fn new_zai_sdk_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        source_channel: Option<&str>,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = ZaiSdkClientConfig {
            workspace,
            model,
            api_key,
            base_url,
            source_channel,
        };
        let components = create_zai_sdk_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

}
