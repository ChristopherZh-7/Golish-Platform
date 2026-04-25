//! OpenAI-compatible providers: OpenRouter, OpenAI (Chat + Responses), NVIDIA NIM.
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
    create_nvidia_components, create_openai_components, create_openrouter_components,
    NvidiaClientConfig, OpenAiClientConfig, OpenRouterClientConfig, SharedComponentsConfig,
};

use super::super::AgentBridge;

impl AgentBridge {

    pub async fn new_openrouter_with_runtime(
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
        Self::new_openrouter_with_shared_config(
            workspace,
            model,
            api_key,
            None,
            shared_config,
            runtime,
            "",
        )
        .await
    }


    pub async fn new_openrouter_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        provider_preferences: Option<serde_json::Value>,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = OpenRouterClientConfig {
            workspace,
            model,
            api_key,
            provider_preferences,
        };

        let components = create_openrouter_components(config, shared_config).await?;

        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }


    pub async fn new_openai_with_runtime(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        reasoning_effort: Option<&str>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        Self::new_openai_with_context(
            workspace,
            model,
            api_key,
            base_url,
            reasoning_effort,
            None,
            runtime,
        )
        .await
    }


    pub async fn new_openai_with_context(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        reasoning_effort: Option<&str>,
        context_config: Option<ContextManagerConfig>,
        runtime: Arc<dyn GolishRuntime>,
    ) -> Result<Self> {
        let shared_config = SharedComponentsConfig {
            context_config,
            settings: golish_settings::GolishSettings::default(),
        };
        Self::new_openai_with_shared_config(
            workspace,
            model,
            api_key,
            base_url,
            reasoning_effort,
            shared_config,
            runtime,
            "",
        )
        .await
    }


    #[allow(clippy::too_many_arguments)]
    pub async fn new_openai_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        reasoning_effort: Option<&str>,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = OpenAiClientConfig {
            workspace,
            model,
            api_key,
            base_url,
            reasoning_effort,
            enable_web_search: false,
            web_search_context_size: "medium",
        };
        let components = create_openai_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }


    pub async fn new_nvidia_with_shared_config(
        workspace: PathBuf,
        model: &str,
        api_key: &str,
        base_url: Option<&str>,
        shared_config: SharedComponentsConfig,
        runtime: Arc<dyn GolishRuntime>,
        event_session_id: &str,
    ) -> Result<Self> {
        let config = NvidiaClientConfig {
            workspace,
            model,
            api_key,
            base_url,
        };
        let components = create_nvidia_components(config, shared_config).await?;
        Ok(Self::from_components_with_runtime(
            components,
            runtime,
            event_session_id.to_string(),
        ))
    }

}
