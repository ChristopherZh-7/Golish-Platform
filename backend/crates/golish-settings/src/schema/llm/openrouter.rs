//! OpenRouter API settings, including the rich provider-preferences block
//! used for routing, filtering, and prioritization.

use super::super::defaults::*;
use serde::{Deserialize, Serialize};

/// OpenRouter API settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenRouterSettings {
    /// OpenRouter API key (supports $ENV_VAR syntax)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Whether to show this provider's models in the model selector
    #[serde(default = "default_true")]
    pub show_in_selector: bool,

    /// Provider preferences for routing and filtering (optional).
    /// See https://openrouter.ai/docs/guides/routing/provider-selection
    #[serde(default, skip_serializing_if = "provider_preferences_is_empty")]
    pub provider_preferences: Option<OpenRouterProviderPreferences>,
}

/// Custom skip-serialization check: skip only if None or all-empty.
/// This ensures non-empty preferences are ALWAYS serialized, preventing
/// data loss when the settings file is rewritten (e.g., on window resize).
fn provider_preferences_is_empty(prefs: &Option<OpenRouterProviderPreferences>) -> bool {
    match prefs {
        None => true,
        Some(p) => p.is_empty(),
    }
}

/// OpenRouter provider preferences for routing, filtering, and prioritization.
///
/// Maps to OpenRouter's Provider Routing API:
/// <https://openrouter.ai/docs/guides/routing/provider-selection>
///
/// All fields are optional. Only non-None fields are sent to the API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenRouterProviderPreferences {
    /// Provider priority ordering. Try these providers first, in order.
    /// Example: ["deepinfra", "deepseek"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,

    /// Hard allowlist: only use these providers.
    /// Example: ["deepinfra", "atlascloud"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub only: Option<Vec<String>>,

    /// Blocklist: never use these providers.
    /// Example: ["google vertex"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore: Option<Vec<String>>,

    /// Whether to allow fallback to other providers when preferred ones are unavailable.
    /// Defaults to true if not specified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_fallbacks: Option<bool>,

    /// Only route to providers that support all request parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_parameters: Option<bool>,

    /// Data collection policy: "allow" or "deny".
    /// "deny" restricts to providers that do not store user data non-transiently.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_collection: Option<String>,

    /// Require Zero Data Retention endpoints only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zdr: Option<bool>,

    /// Sort providers by: "price", "throughput", or "latency".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,

    /// Minimum throughput threshold in tokens/sec.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_min_throughput: Option<f64>,

    /// Maximum latency threshold in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_max_latency: Option<f64>,

    /// Maximum price per prompt token (in USD per million tokens).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_price_prompt: Option<f64>,

    /// Maximum price per completion token (in USD per million tokens).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_price_completion: Option<f64>,

    /// Filter by quantization levels.
    /// Valid values: "int4", "int8", "fp8", "fp16", "bf16", "fp32", "unknown"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantizations: Option<Vec<String>>,
}

impl OpenRouterProviderPreferences {
    /// Check if any preferences are set.
    pub fn is_empty(&self) -> bool {
        self.order.is_none()
            && self.only.is_none()
            && self.ignore.is_none()
            && self.allow_fallbacks.is_none()
            && self.require_parameters.is_none()
            && self.data_collection.is_none()
            && self.zdr.is_none()
            && self.sort.is_none()
            && self.preferred_min_throughput.is_none()
            && self.preferred_max_latency.is_none()
            && self.max_price_prompt.is_none()
            && self.max_price_completion.is_none()
            && self.quantizations.is_none()
    }
}

impl Default for OpenRouterSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            show_in_selector: true,
            provider_preferences: None,
        }
    }
}
