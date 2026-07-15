//! Pure provider/shared-component normalization shared by GUI and CLI bootstrap.

use golish_agent_kit::llm_client::{ContextManagerConfig, ProviderConfig, SharedComponentsConfig};
use golish_settings::schema::ModelOverride;
use golish_settings::GolishSettings;

/// Provider and shared runtime configuration produced as one bootstrap unit.
pub struct AgentBootstrapConfig {
    pub provider_config: ProviderConfig,
    pub shared_components_config: SharedComponentsConfig,
}

/// Normalize one typed adapter input before constructing an `AgentBridge`.
pub fn normalize_agent_bootstrap(
    provider_config: ProviderConfig,
    settings: &GolishSettings,
) -> AgentBootstrapConfig {
    let provider_name = provider_config.provider_name();
    let model = provider_config.model().to_string();
    let settings_model_override = settings
        .ai
        .model_overrides
        .get(&format!("{provider_name}::{model}"))
        .cloned();

    let provider_config = match provider_config {
        ProviderConfig::VertexAi {
            workspace,
            model,
            credentials_path,
            project_id,
            location,
            model_override,
        } => ProviderConfig::VertexAi {
            workspace,
            model,
            credentials_path: prefer_optional_string(
                credentials_path,
                settings.ai.vertex_ai.credentials_path.clone(),
            ),
            project_id: prefer_required_string(
                project_id,
                settings.ai.vertex_ai.project_id.clone(),
                "",
            ),
            location: prefer_required_string(
                location,
                settings.ai.vertex_ai.location.clone(),
                "us-east5",
            ),
            model_override: prefer_model_override(model_override, settings_model_override),
        },
        ProviderConfig::VertexGemini {
            workspace,
            model,
            credentials_path,
            project_id,
            location,
            include_thoughts: _,
            model_override,
        } => ProviderConfig::VertexGemini {
            workspace,
            model,
            credentials_path: prefer_optional_string(
                credentials_path,
                settings.ai.vertex_gemini.credentials_path.clone(),
            ),
            project_id: prefer_required_string(
                project_id,
                settings.ai.vertex_gemini.project_id.clone(),
                "",
            ),
            location: prefer_required_string(
                location,
                settings.ai.vertex_gemini.location.clone(),
                "us-central1",
            ),
            include_thoughts: settings.ai.vertex_gemini.include_thoughts,
            model_override: prefer_model_override(model_override, settings_model_override),
        },
        ProviderConfig::Openrouter {
            workspace,
            model,
            api_key,
            provider_preferences,
            model_override,
        } => ProviderConfig::Openrouter {
            workspace,
            model,
            api_key,
            provider_preferences: provider_preferences.or_else(|| {
                settings
                    .ai
                    .openrouter
                    .provider_preferences
                    .as_ref()
                    .filter(|preferences| !preferences.is_empty())
                    .map(golish_llm_providers::openrouter_preferences_to_json)
            }),
            model_override: prefer_model_override(model_override, settings_model_override),
        },
        ProviderConfig::Openai {
            workspace,
            model,
            api_key,
            base_url,
            reasoning_effort,
            enable_web_search: _,
            web_search_context_size: _,
            model_override,
        } => ProviderConfig::Openai {
            workspace,
            model,
            api_key,
            base_url: prefer_optional_string(base_url, settings.ai.openai.base_url.clone()),
            reasoning_effort: prefer_optional_string(
                reasoning_effort,
                settings
                    .ai
                    .default_reasoning_effort
                    .map(|effort| effort.to_string()),
            ),
            enable_web_search: settings.ai.openai.enable_web_search,
            web_search_context_size: settings.ai.openai.web_search_context_size.clone(),
            model_override: prefer_model_override(model_override, settings_model_override),
        },
        ProviderConfig::Anthropic {
            workspace,
            model,
            api_key,
            model_override,
        } => ProviderConfig::Anthropic {
            workspace,
            model,
            api_key,
            model_override: prefer_model_override(model_override, settings_model_override),
        },
        ProviderConfig::Ollama {
            workspace,
            model,
            base_url,
            model_override,
        } => ProviderConfig::Ollama {
            workspace,
            model,
            base_url: prefer_optional_string(base_url, Some(settings.ai.ollama.base_url.clone())),
            model_override: prefer_model_override(model_override, settings_model_override),
        },
        ProviderConfig::Gemini {
            workspace,
            model,
            api_key,
            include_thoughts: _,
            model_override,
        } => ProviderConfig::Gemini {
            workspace,
            model,
            api_key,
            include_thoughts: settings.ai.gemini.include_thoughts,
            model_override: prefer_model_override(model_override, settings_model_override),
        },
        ProviderConfig::Groq {
            workspace,
            model,
            api_key,
            model_override,
        } => ProviderConfig::Groq {
            workspace,
            model,
            api_key,
            model_override: prefer_model_override(model_override, settings_model_override),
        },
        ProviderConfig::Xai {
            workspace,
            model,
            api_key,
            model_override,
        } => ProviderConfig::Xai {
            workspace,
            model,
            api_key,
            model_override: prefer_model_override(model_override, settings_model_override),
        },
        ProviderConfig::ZaiSdk {
            workspace,
            model,
            api_key,
            base_url,
            source_channel,
            model_override,
        } => ProviderConfig::ZaiSdk {
            workspace,
            model,
            api_key,
            base_url: prefer_optional_string(base_url, settings.ai.zai_sdk.base_url.clone()),
            source_channel,
            model_override: prefer_model_override(model_override, settings_model_override),
        },
        ProviderConfig::Nvidia {
            workspace,
            model,
            api_key,
            base_url,
            model_override,
        } => ProviderConfig::Nvidia {
            workspace,
            model,
            api_key,
            base_url: prefer_optional_string(base_url, settings.ai.nvidia.base_url.clone()),
            model_override: prefer_model_override(model_override, settings_model_override),
        },
        ProviderConfig::Deepseek {
            workspace,
            model,
            api_key,
            base_url,
            model_override,
        } => ProviderConfig::Deepseek {
            workspace,
            model,
            api_key,
            base_url: prefer_optional_string(base_url, settings.ai.deepseek.base_url.clone()),
            model_override: prefer_model_override(model_override, settings_model_override),
        },
        ProviderConfig::Xiaomi {
            workspace,
            model,
            api_key,
            region,
            default_protocol,
            base_url,
            anthropic_base_url,
            model_override,
        } => ProviderConfig::Xiaomi {
            workspace,
            model,
            api_key,
            region: prefer_optional_string(region, settings.ai.xiaomi.region.clone()),
            default_protocol: prefer_optional_string(
                default_protocol,
                settings.ai.xiaomi.default_protocol.clone(),
            ),
            base_url: prefer_optional_string(base_url, settings.ai.xiaomi.openai_base_url.clone()),
            anthropic_base_url: prefer_optional_string(
                anthropic_base_url,
                settings.ai.xiaomi.anthropic_base_url.clone(),
            ),
            model_override: prefer_model_override(model_override, settings_model_override),
        },
    };

    let context_config = settings.context.enabled.then_some(ContextManagerConfig {
        enabled: settings.context.enabled,
        compaction_threshold: settings.context.compaction_threshold,
        protected_turns: settings.context.protected_turns,
        cooldown_seconds: settings.context.cooldown_seconds,
    });

    AgentBootstrapConfig {
        provider_config,
        shared_components_config: SharedComponentsConfig {
            settings: settings.clone(),
            context_config,
        },
    }
}

fn prefer_model_override(
    explicit: Option<ModelOverride>,
    settings: Option<ModelOverride>,
) -> Option<ModelOverride> {
    explicit.or(settings)
}

fn prefer_optional_string(explicit: Option<String>, settings: Option<String>) -> Option<String> {
    explicit
        .filter(|value| !value.trim().is_empty())
        .or_else(|| settings.filter(|value| !value.trim().is_empty()))
}

fn prefer_required_string(explicit: String, settings: Option<String>, default: &str) -> String {
    if explicit.trim().is_empty() {
        settings
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default.to_string())
    } else {
        explicit
    }
}

#[cfg(test)]
mod tests {
    use golish_agent_kit::llm_client::ProviderConfig;
    use golish_settings::schema::{ModelOverride, OpenRouterProviderPreferences, ReasoningEffort};
    use golish_settings::GolishSettings;

    use super::normalize_agent_bootstrap;

    fn override_with_max_tokens(max_tokens: u32) -> ModelOverride {
        ModelOverride {
            max_tokens: Some(max_tokens),
            ..ModelOverride::default()
        }
    }

    #[test]
    fn openai_and_context_settings_are_normalized_once() {
        let mut settings = GolishSettings::default();
        settings.ai.openai.base_url = Some("https://settings.openai.test/v1".to_string());
        settings.ai.default_reasoning_effort = Some(ReasoningEffort::High);
        settings.ai.openai.enable_web_search = true;
        settings.ai.openai.web_search_context_size = "high".to_string();
        settings.ai.model_overrides.insert(
            "openai::gpt-parity".to_string(),
            override_with_max_tokens(12_345),
        );
        settings.context.enabled = true;
        settings.context.compaction_threshold = 0.73;
        settings.context.protected_turns = 7;
        settings.context.cooldown_seconds = 19;

        let normalized = normalize_agent_bootstrap(
            ProviderConfig::Openai {
                workspace: "/tmp/gui".to_string(),
                model: "gpt-parity".to_string(),
                api_key: "gui-key".to_string(),
                base_url: None,
                reasoning_effort: None,
                enable_web_search: false,
                web_search_context_size: "medium".to_string(),
                model_override: None,
            },
            &settings,
        );

        match normalized.provider_config {
            ProviderConfig::Openai {
                workspace,
                model,
                api_key,
                base_url,
                reasoning_effort,
                enable_web_search,
                web_search_context_size,
                model_override,
            } => {
                assert_eq!(workspace, "/tmp/gui");
                assert_eq!(model, "gpt-parity");
                assert_eq!(api_key, "gui-key");
                assert_eq!(base_url.as_deref(), Some("https://settings.openai.test/v1"));
                assert_eq!(reasoning_effort.as_deref(), Some("high"));
                assert!(enable_web_search);
                assert_eq!(web_search_context_size, "high");
                assert_eq!(model_override, Some(override_with_max_tokens(12_345)));
            }
            other => panic!("expected OpenAI config, got {other:?}"),
        }

        let context = normalized
            .shared_components_config
            .context_config
            .expect("enabled settings must produce shared context config");
        assert!(context.enabled);
        assert_eq!(context.compaction_threshold, 0.73);
        assert_eq!(context.protected_turns, 7);
        assert_eq!(context.cooldown_seconds, 19);
    }

    #[test]
    fn explicit_gui_provider_fields_win_over_settings_fallbacks() {
        let mut settings = GolishSettings::default();
        settings.ai.openai.base_url = Some("https://settings.test/v1".to_string());
        settings.ai.default_reasoning_effort = Some(ReasoningEffort::Low);
        settings.ai.model_overrides.insert(
            "openai::gpt-explicit".to_string(),
            override_with_max_tokens(1_000),
        );

        let normalized = normalize_agent_bootstrap(
            ProviderConfig::Openai {
                workspace: "/tmp/gui".to_string(),
                model: "gpt-explicit".to_string(),
                api_key: "explicit-key".to_string(),
                base_url: Some("https://explicit.test/v1".to_string()),
                reasoning_effort: Some("extra_high".to_string()),
                enable_web_search: false,
                web_search_context_size: "medium".to_string(),
                model_override: Some(override_with_max_tokens(9_999)),
            },
            &settings,
        );

        assert!(matches!(
            normalized.provider_config,
            ProviderConfig::Openai {
                api_key,
                base_url: Some(base_url),
                reasoning_effort: Some(reasoning_effort),
                model_override: Some(model_override),
                ..
            } if api_key == "explicit-key"
                && base_url == "https://explicit.test/v1"
                && reasoning_effort == "extra_high"
                && model_override.max_tokens == Some(9_999)
        ));
    }

    #[test]
    fn openrouter_preferences_use_explicit_value_then_settings_fallback() {
        let mut settings = GolishSettings::default();
        settings.ai.openrouter.provider_preferences = Some(OpenRouterProviderPreferences {
            order: Some(vec!["settings-provider".to_string()]),
            ..OpenRouterProviderPreferences::default()
        });
        let expected_settings = golish_llm_providers::openrouter_preferences_to_json(
            settings
                .ai
                .openrouter
                .provider_preferences
                .as_ref()
                .expect("settings preferences"),
        );

        let fallback = normalize_agent_bootstrap(
            ProviderConfig::Openrouter {
                workspace: "/tmp/gui".to_string(),
                model: "router-model".to_string(),
                api_key: "router-key".to_string(),
                provider_preferences: None,
                model_override: None,
            },
            &settings,
        );
        assert!(matches!(
            fallback.provider_config,
            ProviderConfig::Openrouter { provider_preferences: Some(value), .. }
                if value == expected_settings
        ));

        let explicit = serde_json::json!({"provider": {"order": ["explicit-provider"]}});
        let preserved = normalize_agent_bootstrap(
            ProviderConfig::Openrouter {
                workspace: "/tmp/gui".to_string(),
                model: "router-model".to_string(),
                api_key: "router-key".to_string(),
                provider_preferences: Some(explicit.clone()),
                model_override: None,
            },
            &settings,
        );
        assert!(matches!(
            preserved.provider_config,
            ProviderConfig::Openrouter { provider_preferences: Some(value), .. }
                if value == explicit
        ));
    }

    #[test]
    fn google_and_ollama_hidden_settings_are_shared() {
        let mut settings = GolishSettings::default();
        settings.ai.vertex_ai.location = Some("europe-west1".to_string());
        settings.ai.vertex_gemini.location = Some("asia-northeast1".to_string());
        settings.ai.vertex_gemini.include_thoughts = true;
        settings.ai.gemini.include_thoughts = true;
        settings.ai.ollama.base_url = "http://ollama.settings:11434".to_string();

        let vertex_ai = normalize_agent_bootstrap(
            ProviderConfig::VertexAi {
                workspace: "/tmp/gui".to_string(),
                model: "claude".to_string(),
                credentials_path: None,
                project_id: "project".to_string(),
                location: String::new(),
                model_override: None,
            },
            &settings,
        );
        assert!(matches!(
            vertex_ai.provider_config,
            ProviderConfig::VertexAi { location, .. } if location == "europe-west1"
        ));

        let vertex_gemini = normalize_agent_bootstrap(
            ProviderConfig::VertexGemini {
                workspace: "/tmp/gui".to_string(),
                model: "gemini".to_string(),
                credentials_path: None,
                project_id: "project".to_string(),
                location: String::new(),
                include_thoughts: false,
                model_override: None,
            },
            &settings,
        );
        assert!(matches!(
            vertex_gemini.provider_config,
            ProviderConfig::VertexGemini { location, include_thoughts: true, .. }
                if location == "asia-northeast1"
        ));

        let gemini = normalize_agent_bootstrap(
            ProviderConfig::Gemini {
                workspace: "/tmp/gui".to_string(),
                model: "gemini".to_string(),
                api_key: "gemini-key".to_string(),
                include_thoughts: false,
                model_override: None,
            },
            &settings,
        );
        assert!(matches!(
            gemini.provider_config,
            ProviderConfig::Gemini {
                include_thoughts: true,
                ..
            }
        ));

        let ollama = normalize_agent_bootstrap(
            ProviderConfig::Ollama {
                workspace: "/tmp/gui".to_string(),
                model: "llama".to_string(),
                base_url: None,
                model_override: None,
            },
            &settings,
        );
        assert!(matches!(
            ollama.provider_config,
            ProviderConfig::Ollama { base_url: Some(base_url), .. }
                if base_url == "http://ollama.settings:11434"
        ));
    }

    #[test]
    fn disabled_context_settings_remain_disabled_for_both_adapters() {
        let mut settings = GolishSettings::default();
        settings.context.enabled = false;

        let normalized = normalize_agent_bootstrap(
            ProviderConfig::Ollama {
                workspace: "/tmp/gui".to_string(),
                model: "llama".to_string(),
                base_url: None,
                model_override: None,
            },
            &settings,
        );

        assert!(normalized.shared_components_config.context_config.is_none());
    }
}
