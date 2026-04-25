//! Late-stage override of the reasoning config from
//! `additional_params["reasoning"]`. The agentic loop uses this to
//! tweak effort/summary per call without conflicting with the
//! initialiser values on the model struct.

use async_openai::types::responses::{
    Reasoning, ReasoningEffort as OAReasoningEffort, ReasoningSummary,
};

/// Merge the `"reasoning"` object from `additional_params` into a base
/// reasoning config.
///
/// The agentic loop places a `"reasoning"` key into `additional_params`
/// to control effort and summary level independently of what the model
/// struct was initialised with. This function applies those overrides
/// on top of the base config built from the model struct.
///
/// Rules:
/// - `effort` from params overrides the model-struct default (or `None`).
/// - `summary` from params overrides the `Detailed` default.
/// - Unknown or invalid string values are silently ignored; the
///   existing value is kept.
/// - If `additional_params` has no `"reasoning"` key the base config is
///   returned unchanged.
pub(crate) fn apply_additional_params_reasoning(
    base: Option<Reasoning>,
    additional_params: Option<&serde_json::Value>,
) -> Option<Reasoning> {
    let Some(params) = additional_params else {
        return base;
    };
    let Some(reasoning_json) = params.get("reasoning") else {
        return base;
    };

    let override_effort = reasoning_json
        .get("effort")
        .and_then(|v| v.as_str())
        .and_then(|s| match s {
            "low" => Some(OAReasoningEffort::Low),
            "medium" => Some(OAReasoningEffort::Medium),
            "high" => Some(OAReasoningEffort::High),
            "extra_high" | "xhigh" => Some(OAReasoningEffort::Xhigh),
            _ => None, // unknown values ignored
        });

    let override_summary = reasoning_json
        .get("summary")
        .and_then(|v| v.as_str())
        .and_then(|s| match s {
            "auto" => Some(ReasoningSummary::Auto),
            "concise" => Some(ReasoningSummary::Concise),
            "detailed" => Some(ReasoningSummary::Detailed),
            _ => None, // unknown values ignored
        });

    if override_effort.is_none() && override_summary.is_none() {
        return base;
    }

    let current = base.unwrap_or(Reasoning {
        effort: None,
        summary: None,
    });
    Some(Reasoning {
        effort: override_effort.or(current.effort),
        summary: override_summary.or(current.summary),
    })
}
