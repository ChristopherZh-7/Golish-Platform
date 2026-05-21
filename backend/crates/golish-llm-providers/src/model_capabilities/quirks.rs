//! Provider-level stream quirks: how to interpret SSE chunks for a given
//! `(provider, model)` pair.
//!
//! # Background
//!
//! Different LLM providers (and even the same model deployed across different
//! providers) interpret the OpenAI-compatible streaming protocol in subtly
//! incompatible ways. The most painful divergence today is around the
//! non-standard `reasoning_content` field, which:
//!
//! - **NVIDIA NIM (vLLM-backed)** uses for Qwen3 chain-of-thought.
//!   `enable_thinking=false` does *not* reliably suppress it
//!   ([vllm-project/vllm#40816]). Worse, vLLM may misplace the *final answer*
//!   in `reasoning_content` while leaving `content` empty.
//! - **DashScope (official Qwen API)** strictly separates `reasoning_content`
//!   from `content` and supports a top-level `enable_thinking` flag.
//! - **OpenRouter** sometimes renames the field to `reasoning` (newer vLLM)
//!   and exposes a `reasoning.exclude` switch.
//! - **OpenAI o-series** uses a completely different protocol (Responses API,
//!   `rs_xxx` IDs, encrypted reasoning content).
//!
//! Without a quirks layer, the agent ends up displaying the model's *final
//! answer* inside a `ThinkingBlock`, which is what the user sees in the
//! reported bug for `nvidia/qwen/qwen3.5-122b-a10b`.
//!
//! # Resolution flow
//!
//! [`resolve_stream_quirks`] is the single entry point:
//!
//! 1. Start from a provider-level default (e.g. NVIDIA → `AlwaysContent` for
//!    non-thinking models, `Standard` for explicit thinking models like
//!    DeepSeek R1 / Kimi K2 Thinking).
//! 2. Apply user-supplied `ModelOverride` (per `(provider, model)`) on top.
//!    A user toggling Thinking *on* for a Qwen3 model upgrades the quirks to
//!    `Standard` plus a request-side `enable_thinking=true` kwarg.
//!
//! The struct intentionally describes *behavior* (how to handle the chunk),
//! not *capability* (whether the model can think). The latter lives in
//! [`ModelCapabilities`][crate::ModelCapabilities] and is the input to
//! resolving the default quirks.
//!
//! [vllm-project/vllm#40816]: https://github.com/vllm-project/vllm/issues/40816

use serde::{Deserialize, Serialize};

pub use golish_settings::schema::ModelOverride;

/// How a streaming `reasoning_content` (or `reasoning`) chunk should be
/// surfaced to the rest of the agent pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningHandling {
    /// Treat as proper thinking: emit `AiEvent::Reasoning` and (if the model
    /// supports thinking history) accumulate into the thinking buffer.
    ///
    /// Used by Anthropic extended thinking, OpenAI o-series, DeepSeek R1,
    /// Kimi K2 Thinking, GLM-4.7, etc.
    Standard,

    /// Treat reasoning chunks as thinking *unless* the final `content` field
    /// stays empty — in which case the buffered reasoning is replayed as
    /// regular text on stream end.
    ///
    /// Used as a defensive default for provider/model pairs known to
    /// occasionally misplace the final answer in `reasoning_content`
    /// (the vLLM Qwen3 bug class).
    FallbackToContent,

    /// Never trust reasoning chunks. Re-emit every reasoning fragment as a
    /// regular `AiEvent::TextDelta` and never populate the thinking buffer.
    ///
    /// Used when the user has *explicitly* disabled thinking for a hybrid
    /// model, or when the provider is known to always misroute the final
    /// answer into `reasoning_content` (e.g. NVIDIA NIM serving Qwen3 with
    /// the default `enable_thinking=true`).
    AlwaysContent,
}

/// Where (if anywhere) to set `enable_thinking=false` on the outgoing request
/// so the provider stops emitting reasoning chunks at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingDisableField {
    /// The provider has no documented switch; we can't disable from the
    /// request side and must rely on `ReasoningHandling` at parse time.
    None,
    /// vLLM-style: `chat_template_kwargs: { "enable_thinking": false }`.
    /// Used by NVIDIA NIM, Together AI, most vLLM deployments.
    ChatTemplateKwargs,
    /// DashScope-style: top-level `enable_thinking: false`.
    TopLevelEnableThinking,
    /// OpenRouter / Anthropic-style: `reasoning: { "exclude": true }`.
    OpenRouterReasoningExclude,
}

/// Aggregate description of how a `(provider, model)` pair streams reasoning
/// content and how to suppress that reasoning when the user doesn't want it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStreamQuirks {
    /// JSON field names that may carry hidden reasoning. Listed in priority
    /// order; the deserialization layer should accept any of them.
    pub reasoning_field_aliases: Vec<String>,

    /// How to surface a reasoning chunk to downstream consumers.
    pub reasoning_handling: ReasoningHandling,

    /// Where to inject the `enable_thinking` flag in the outgoing request
    /// body. The actual value comes from
    /// [`thinking_kwargs_value`](Self::thinking_kwargs_value).
    pub disable_thinking_field: ThinkingDisableField,

    /// Whether the resolver applied this configuration *because the user
    /// disabled thinking* (versus because the model never supported it).
    /// Used by the request builder to decide whether to inject the disable
    /// kwarg and by traces / debug overlay to attribute the decision.
    pub force_disable_thinking_kwargs: bool,

    /// Optional explicit value to inject for `enable_thinking` / equivalent.
    /// `Some(true)` forces thinking on; `Some(false)` forces thinking off;
    /// `None` means "don't inject anything; let the provider's own default
    /// apply".
    pub thinking_kwargs_value: Option<bool>,
}

impl ProviderStreamQuirks {
    /// Conservative default: trust the provider, no request-side suppression.
    /// Used for providers / models we have no special knowledge of.
    pub fn standard() -> Self {
        Self {
            reasoning_field_aliases: vec!["reasoning_content".into(), "reasoning".into()],
            reasoning_handling: ReasoningHandling::Standard,
            disable_thinking_field: ThinkingDisableField::None,
            force_disable_thinking_kwargs: false,
            thinking_kwargs_value: None,
        }
    }

    /// Defensive default: treat reasoning as thinking but fall back to text
    /// if the final `content` is empty. Used for OpenRouter / unknown vLLM
    /// deployments that might exhibit the Qwen3 bug.
    pub fn fallback_to_content() -> Self {
        Self {
            reasoning_handling: ReasoningHandling::FallbackToContent,
            ..Self::standard()
        }
    }

    /// True if the resulting quirks should accumulate reasoning chunks into
    /// the thinking buffer (and emit `AiEvent::Reasoning`). When the user
    /// has explicitly toggled thinking on, we route reasoning to the
    /// thinking pane regardless of whether the model's intrinsic
    /// `supports_thinking_history` capability is set.
    pub fn route_reasoning_to_thinking(&self) -> bool {
        matches!(
            self.reasoning_handling,
            ReasoningHandling::Standard | ReasoningHandling::FallbackToContent
        )
    }
}

/// Resolve the effective stream quirks for the given `(provider, model)`
/// pair, applying any user `ModelOverride` on top of the provider default.
///
/// This is the single place where provider/model-specific stream knowledge
/// is encoded. Add new model families by extending the `match` arms below.
pub fn resolve_stream_quirks(
    provider: &str,
    model: &str,
    user_override: Option<&ModelOverride>,
) -> ProviderStreamQuirks {
    let mut quirks = default_quirks_for(provider, model);

    if let Some(over) = user_override {
        if let Some(user_thinking) = over.thinking {
            if user_thinking {
                quirks.reasoning_handling = ReasoningHandling::Standard;
                quirks.force_disable_thinking_kwargs = false;
                // Explicitly ask the provider to enable thinking instead of
                // relying on its default; some NIM / vLLM deployments ship
                // with `enable_thinking=false` baked into the chat template.
                if !matches!(quirks.disable_thinking_field, ThinkingDisableField::None) {
                    quirks.thinking_kwargs_value = Some(true);
                }
            } else {
                quirks.reasoning_handling = ReasoningHandling::AlwaysContent;
                if !matches!(quirks.disable_thinking_field, ThinkingDisableField::None) {
                    quirks.force_disable_thinking_kwargs = true;
                    quirks.thinking_kwargs_value = Some(false);
                }
            }
        }
    }

    // If we inherited a provider default that forces thinking off (e.g. the
    // NVIDIA Qwen3 hybrid case) and the user did NOT supply an override, the
    // disable value still needs to be `false` so the request injection picks
    // it up.
    if quirks.thinking_kwargs_value.is_none() && quirks.force_disable_thinking_kwargs {
        quirks.thinking_kwargs_value = Some(false);
    }

    quirks
}

/// Provider-default quirks: what we believe the model does *out of the box*,
/// before any user override.
fn default_quirks_for(provider: &str, model: &str) -> ProviderStreamQuirks {
    let model_lower = model.to_lowercase();

    match provider {
        "nvidia" | "nvidia_nim" | "nim" => nvidia_default_quirks(&model_lower),

        // OpenRouter routes to many backends; vLLM ones may exhibit Qwen3 bug.
        // Use FallbackToContent so we don't lose the answer if it misroutes.
        "openrouter" => ProviderStreamQuirks {
            disable_thinking_field: ThinkingDisableField::OpenRouterReasoningExclude,
            ..ProviderStreamQuirks::fallback_to_content()
        },

        // Anthropic, OpenAI Responses, Vertex Gemini, Z.AI native: trusted.
        _ => ProviderStreamQuirks::standard(),
    }
}

/// NVIDIA NIM defaults.
///
/// vLLM is the backbone; Qwen3 family is hybrid-thinking and emits reasoning
/// chunks by default. We list explicit thinking models first (trusted) and
/// fall through to `AlwaysContent + ChatTemplateKwargs` for everything else
/// that might have the misroute bug.
fn nvidia_default_quirks(model_lower: &str) -> ProviderStreamQuirks {
    let is_explicit_thinking_model = model_lower.contains("kimi-k2-thinking")
        || model_lower.contains("deepseek-r1")
        || model_lower.contains("deepseek-v3.2")
        || model_lower.contains("phi-4-mini-flash-reasoning")
        || model_lower.contains("step-3.5-flash")
        || model_lower.contains("qwq")
        || (model_lower.contains("qwen3") && model_lower.contains("thinking"));

    if is_explicit_thinking_model {
        return ProviderStreamQuirks {
            disable_thinking_field: ThinkingDisableField::ChatTemplateKwargs,
            ..ProviderStreamQuirks::standard()
        };
    }

    let is_qwen3_hybrid = model_lower.contains("qwen3") || model_lower.contains("qwen/qwen3");
    if is_qwen3_hybrid {
        return ProviderStreamQuirks {
            reasoning_handling: ReasoningHandling::AlwaysContent,
            disable_thinking_field: ThinkingDisableField::ChatTemplateKwargs,
            force_disable_thinking_kwargs: true,
            ..ProviderStreamQuirks::standard()
        };
    }

    ProviderStreamQuirks {
        reasoning_handling: ReasoningHandling::FallbackToContent,
        disable_thinking_field: ThinkingDisableField::ChatTemplateKwargs,
        ..ProviderStreamQuirks::standard()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_default_is_standard() {
        let q = resolve_stream_quirks("anthropic", "claude-3-opus", None);
        assert_eq!(q.reasoning_handling, ReasoningHandling::Standard);
        assert!(!q.force_disable_thinking_kwargs);
        assert_eq!(q.thinking_kwargs_value, None);
    }

    #[test]
    fn nvidia_qwen3_default_is_always_content() {
        let q = resolve_stream_quirks("nvidia", "qwen/qwen3.5-122b-a10b", None);
        assert_eq!(q.reasoning_handling, ReasoningHandling::AlwaysContent);
        assert!(q.force_disable_thinking_kwargs);
        assert_eq!(q.thinking_kwargs_value, Some(false));
        assert_eq!(
            q.disable_thinking_field,
            ThinkingDisableField::ChatTemplateKwargs
        );
    }

    #[test]
    fn nvidia_kimi_k2_thinking_default_is_standard() {
        let q = resolve_stream_quirks("nvidia", "moonshotai/kimi-k2-thinking", None);
        assert_eq!(q.reasoning_handling, ReasoningHandling::Standard);
        assert!(!q.force_disable_thinking_kwargs);
        assert_eq!(q.thinking_kwargs_value, None);
    }

    #[test]
    fn nvidia_deepseek_r1_default_is_standard() {
        let q = resolve_stream_quirks("nvidia", "deepseek-ai/deepseek-r1-distill-qwen-32b", None);
        assert_eq!(q.reasoning_handling, ReasoningHandling::Standard);
    }

    #[test]
    fn user_enables_thinking_on_qwen3_forces_enable_thinking_true() {
        let over = ModelOverride {
            thinking: Some(true),
            ..ModelOverride::default()
        };
        let q = resolve_stream_quirks("nvidia", "qwen/qwen3.5-122b-a10b", Some(&over));
        assert_eq!(q.reasoning_handling, ReasoningHandling::Standard);
        assert!(!q.force_disable_thinking_kwargs);
        assert_eq!(
            q.thinking_kwargs_value,
            Some(true),
            "user opt-in to thinking must force enable_thinking=true on the request"
        );
        assert_eq!(
            q.disable_thinking_field,
            ThinkingDisableField::ChatTemplateKwargs
        );
    }

    #[test]
    fn user_disables_thinking_on_kimi_k2_routes_to_always_content() {
        let over = ModelOverride {
            thinking: Some(false),
            ..ModelOverride::default()
        };
        let q = resolve_stream_quirks("nvidia", "moonshotai/kimi-k2-thinking", Some(&over));
        assert_eq!(q.reasoning_handling, ReasoningHandling::AlwaysContent);
        assert!(q.force_disable_thinking_kwargs);
        assert_eq!(q.thinking_kwargs_value, Some(false));
    }

    #[test]
    fn openrouter_default_is_fallback_to_content() {
        let q = resolve_stream_quirks("openrouter", "qwen/qwen3.5-122b", None);
        assert_eq!(q.reasoning_handling, ReasoningHandling::FallbackToContent);
        assert_eq!(
            q.disable_thinking_field,
            ThinkingDisableField::OpenRouterReasoningExclude
        );
    }

    #[test]
    fn override_default_serializes_to_empty_object() {
        let json = serde_json::to_string(&ModelOverride::default()).unwrap();
        assert_eq!(json, "{}", "default override should serialize to empty");
    }

    #[test]
    fn override_with_thinking_disabled_round_trips() {
        let over = ModelOverride {
            thinking: Some(false),
            ..ModelOverride::default()
        };
        let json = serde_json::to_string(&over).unwrap();
        let back: ModelOverride = serde_json::from_str(&json).unwrap();
        assert_eq!(over, back);
        assert_eq!(over.thinking, Some(false));
    }
}
