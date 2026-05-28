//! Xiaomi MiMo Token Plan provider constants and protocol helpers.
//!
//! See `docs/design/2026-05-27-add-xiaomi-mimo-provider.md` for the upstream
//! facts. Three clusters share one API key; OpenAI-compatible and
//! Anthropic-compatible endpoints live under different path prefixes per
//! cluster.

/// Endpoint cluster for Xiaomi MiMo.
///
/// Token Plan (`tp-…` keys) uses three regional clusters; pay-as-you-go
/// (`sk-…` keys) ships from a single global endpoint. The `from_settings`
/// helper accepts a handful of aliases so users can match the platform's
/// official terminology even when it drifts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum XiaomiRegion {
    /// Token Plan China cluster — `token-plan-cn.xiaomimimo.com`.
    #[default]
    Cn,
    /// Token Plan Singapore cluster — `token-plan-sgp.xiaomimimo.com`.
    Sgp,
    /// Token Plan Amsterdam (Europe) cluster — `token-plan-ams.xiaomimimo.com`.
    Ams,
    /// Pay-as-you-go (sk-… keys) — `api.xiaomimimo.com`.
    ///
    /// Verified live on 2026-05-27: pay-as-you-go keys are rejected with HTTP
    /// 401 on Token Plan endpoints but accepted (modulo billing) on
    /// `api.xiaomimimo.com`.
    PayAsYouGo,
}

impl XiaomiRegion {
    /// Parse a region tag from settings (case-insensitive). Unknown values
    /// fall back to [`XiaomiRegion::Cn`].
    pub fn from_settings(value: Option<&str>) -> Self {
        match value.map(str::to_ascii_lowercase).as_deref() {
            Some("sgp") | Some("singapore") => XiaomiRegion::Sgp,
            Some("ams") | Some("amsterdam") | Some("eu") | Some("europe") => XiaomiRegion::Ams,
            Some("pay_as_you_go")
            | Some("pay-as-you-go")
            | Some("payg")
            | Some("direct")
            | Some("global") => XiaomiRegion::PayAsYouGo,
            _ => XiaomiRegion::Cn,
        }
    }

    /// Default OpenAI-compatible base URL for this region.
    ///
    /// rig-core's OpenAI Chat Completions client appends `/chat/completions` to
    /// the configured base URL, so the trailing `/v1` is required.
    pub fn openai_base_url(self) -> &'static str {
        match self {
            XiaomiRegion::Cn => "https://token-plan-cn.xiaomimimo.com/v1",
            XiaomiRegion::Sgp => "https://token-plan-sgp.xiaomimimo.com/v1",
            XiaomiRegion::Ams => "https://token-plan-ams.xiaomimimo.com/v1",
            XiaomiRegion::PayAsYouGo => "https://api.xiaomimimo.com/v1",
        }
    }

    /// Default Anthropic-compatible base URL for this region.
    ///
    /// rig-core's `normalize_anthropic_base_url` strips a trailing `/v1` or
    /// `/v1/messages`, so we hand it the bare `/anthropic` prefix and the
    /// client re-appends `/v1/messages` at request time.
    pub fn anthropic_base_url(self) -> &'static str {
        match self {
            XiaomiRegion::Cn => "https://token-plan-cn.xiaomimimo.com/anthropic",
            XiaomiRegion::Sgp => "https://token-plan-sgp.xiaomimimo.com/anthropic",
            XiaomiRegion::Ams => "https://token-plan-ams.xiaomimimo.com/anthropic",
            XiaomiRegion::PayAsYouGo => "https://api.xiaomimimo.com/anthropic",
        }
    }
}

/// Wire protocol selection for a Xiaomi model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum XiaomiProtocol {
    /// OpenAI-compatible Chat Completions endpoint.
    OpenaiCompatible,
    /// Anthropic-compatible Messages endpoint.
    AnthropicCompatible,
    /// Honor model id suffix / registry hint; fall back to OpenAI-compatible.
    #[default]
    Auto,
}

impl XiaomiProtocol {
    /// Parse the protocol from settings (case-insensitive). Unknown values
    /// fall back to [`XiaomiProtocol::Auto`].
    pub fn from_settings(value: Option<&str>) -> Self {
        match value.map(str::to_ascii_lowercase).as_deref() {
            Some("openai") | Some("openai_compatible") => XiaomiProtocol::OpenaiCompatible,
            Some("anthropic") | Some("anthropic_compatible") => XiaomiProtocol::AnthropicCompatible,
            _ => XiaomiProtocol::Auto,
        }
    }
}

/// Resolved protocol for `model`, honoring (in order): explicit `@suffix` on
/// the model id, the provider-level `default_protocol`, then OpenAI-compatible
/// as a safe fallback.
///
/// The function is intentionally pure and deterministic so it can be unit
/// tested without touching network or settings disk.
pub fn resolve_protocol(model: &str, default: XiaomiProtocol) -> XiaomiProtocol {
    if model.ends_with("@anthropic") {
        return XiaomiProtocol::AnthropicCompatible;
    }
    if model.ends_with("@openai") {
        return XiaomiProtocol::OpenaiCompatible;
    }
    match default {
        XiaomiProtocol::OpenaiCompatible => XiaomiProtocol::OpenaiCompatible,
        XiaomiProtocol::AnthropicCompatible => XiaomiProtocol::AnthropicCompatible,
        XiaomiProtocol::Auto => XiaomiProtocol::OpenaiCompatible,
    }
}

/// Strip a trailing protocol suffix (`@openai` / `@anthropic`) from the model
/// id, returning the canonical model id to pass to the upstream API.
///
/// Xiaomi's wire-level model id does not include the protocol marker; the
/// suffix is a Golish-side routing hint.
pub fn strip_protocol_suffix(model: &str) -> &str {
    model
        .strip_suffix("@anthropic")
        .or_else(|| model.strip_suffix("@openai"))
        .unwrap_or(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_parses_known_tags() {
        assert_eq!(XiaomiRegion::from_settings(Some("cn")), XiaomiRegion::Cn);
        assert_eq!(XiaomiRegion::from_settings(Some("CN")), XiaomiRegion::Cn);
        assert_eq!(XiaomiRegion::from_settings(Some("sgp")), XiaomiRegion::Sgp);
        assert_eq!(
            XiaomiRegion::from_settings(Some("singapore")),
            XiaomiRegion::Sgp
        );
        assert_eq!(XiaomiRegion::from_settings(Some("ams")), XiaomiRegion::Ams);
        assert_eq!(XiaomiRegion::from_settings(Some("eu")), XiaomiRegion::Ams);
        assert_eq!(
            XiaomiRegion::from_settings(Some("pay_as_you_go")),
            XiaomiRegion::PayAsYouGo
        );
        assert_eq!(
            XiaomiRegion::from_settings(Some("payg")),
            XiaomiRegion::PayAsYouGo
        );
        assert_eq!(
            XiaomiRegion::from_settings(Some("global")),
            XiaomiRegion::PayAsYouGo
        );
    }

    #[test]
    fn region_unknown_falls_back_to_cn() {
        assert_eq!(XiaomiRegion::from_settings(None), XiaomiRegion::Cn);
        assert_eq!(XiaomiRegion::from_settings(Some("mars")), XiaomiRegion::Cn);
    }

    #[test]
    fn region_openai_urls_have_v1_suffix() {
        for region in [
            XiaomiRegion::Cn,
            XiaomiRegion::Sgp,
            XiaomiRegion::Ams,
            XiaomiRegion::PayAsYouGo,
        ] {
            assert!(
                region.openai_base_url().ends_with("/v1"),
                "{region:?} OpenAI base url must end with /v1"
            );
        }
    }

    #[test]
    fn region_anthropic_urls_have_anthropic_suffix() {
        for region in [
            XiaomiRegion::Cn,
            XiaomiRegion::Sgp,
            XiaomiRegion::Ams,
            XiaomiRegion::PayAsYouGo,
        ] {
            assert!(
                region.anthropic_base_url().ends_with("/anthropic"),
                "{region:?} Anthropic base url must end with /anthropic"
            );
        }
    }

    #[test]
    fn pay_as_you_go_region_uses_api_xiaomimimo_endpoint() {
        // Verified live on 2026-05-27 — keep this assertion to guard against
        // accidental refactors that would re-route sk- keys to the Token Plan
        // domain (which 401s them).
        assert_eq!(
            XiaomiRegion::PayAsYouGo.openai_base_url(),
            "https://api.xiaomimimo.com/v1"
        );
        assert_eq!(
            XiaomiRegion::PayAsYouGo.anthropic_base_url(),
            "https://api.xiaomimimo.com/anthropic"
        );
    }

    #[test]
    fn protocol_parses_known_tags() {
        assert_eq!(
            XiaomiProtocol::from_settings(Some("openai")),
            XiaomiProtocol::OpenaiCompatible
        );
        assert_eq!(
            XiaomiProtocol::from_settings(Some("anthropic")),
            XiaomiProtocol::AnthropicCompatible
        );
        assert_eq!(
            XiaomiProtocol::from_settings(Some("auto")),
            XiaomiProtocol::Auto
        );
    }

    #[test]
    fn resolve_protocol_honors_suffix() {
        assert_eq!(
            resolve_protocol("mimo-v2.5-pro@anthropic", XiaomiProtocol::OpenaiCompatible),
            XiaomiProtocol::AnthropicCompatible,
            "@anthropic suffix overrides openai default"
        );
        assert_eq!(
            resolve_protocol("mimo-v2.5-pro@openai", XiaomiProtocol::AnthropicCompatible),
            XiaomiProtocol::OpenaiCompatible,
            "@openai suffix overrides anthropic default"
        );
    }

    #[test]
    fn resolve_protocol_falls_back_to_default_then_openai() {
        assert_eq!(
            resolve_protocol("mimo-v2.5-pro", XiaomiProtocol::AnthropicCompatible),
            XiaomiProtocol::AnthropicCompatible
        );
        assert_eq!(
            resolve_protocol("mimo-v2.5-pro", XiaomiProtocol::Auto),
            XiaomiProtocol::OpenaiCompatible,
            "Auto without registry hint falls back to OpenAI-compatible"
        );
    }

    #[test]
    fn strip_protocol_suffix_strips_known_tags() {
        assert_eq!(
            strip_protocol_suffix("mimo-v2.5-pro@anthropic"),
            "mimo-v2.5-pro"
        );
        assert_eq!(
            strip_protocol_suffix("mimo-v2.5-pro@openai"),
            "mimo-v2.5-pro"
        );
        assert_eq!(strip_protocol_suffix("mimo-v2.5-pro"), "mimo-v2.5-pro");
    }
}
