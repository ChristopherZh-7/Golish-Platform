//! Environment-variable interpolation + proxy/env fallback helpers.
//!
//! Settings file fields can reference `$ENV_VAR` or `${ENV_VAR}`; this
//! module resolves those references at load time and exposes the
//! `get_with_env_fallback` helper for callers that need to layer
//! settings → env → default for a single string field.

use crate::schema::GolishSettings;

/// Resolve a `$ENV_VAR` or `${ENV_VAR}` reference.
///
/// Returns `Some(resolved)` if the value starts with `$` and the env var exists.
/// Returns `None` if no env var reference or env var not set.
pub(super) fn resolve_env_ref(value: &str) -> Option<String> {
    let trimmed = value.trim();

    if let Some(stripped) = trimmed.strip_prefix('$') {
        let var_name = if trimmed.starts_with("${") && trimmed.ends_with('}') {
            // ${VAR_NAME} format
            &trimmed[2..trimmed.len() - 1]
        } else {
            // $VAR_NAME format
            stripped
        };

        return std::env::var(var_name).ok();
    }

    None
}

/// Resolve `$ENV_VAR` references in every relevant string field of `settings`.
///
/// Mutates `settings` in place. Called by [`super::SettingsManager::load_from_path`]
/// after deserialisation.
pub(super) fn resolve_env_vars(settings: &mut GolishSettings) {
    fn resolve_opt(value: &mut Option<String>) {
        if let Some(v) = value {
            if let Some(resolved) = resolve_env_ref(v) {
                *v = resolved;
            }
        }
    }

    // AI settings
    resolve_opt(&mut settings.ai.vertex_ai.credentials_path);
    resolve_opt(&mut settings.ai.vertex_ai.project_id);
    resolve_opt(&mut settings.ai.vertex_ai.location);
    resolve_opt(&mut settings.ai.openrouter.api_key);
    resolve_opt(&mut settings.ai.anthropic.api_key);
    resolve_opt(&mut settings.ai.openai.api_key);
    resolve_opt(&mut settings.ai.openai.base_url);
    resolve_opt(&mut settings.ai.nvidia.api_key);
    resolve_opt(&mut settings.ai.nvidia.base_url);

    // Network settings
    resolve_opt(&mut settings.network.proxy_url);
    resolve_opt(&mut settings.network.no_proxy);

    // API keys
    resolve_opt(&mut settings.api_keys.tavily);
    resolve_opt(&mut settings.api_keys.github);

    // Telemetry settings (Langfuse)
    resolve_opt(&mut settings.telemetry.langfuse.host);
    resolve_opt(&mut settings.telemetry.langfuse.public_key);
    resolve_opt(&mut settings.telemetry.langfuse.secret_key);

    // MCP server env vars
    for config in settings.mcp_servers.values_mut() {
        for v in config.env.values_mut() {
            if let Some(resolved) = resolve_env_ref(v) {
                *v = resolved;
            }
        }
    }
}

/// Get a setting value with environment variable fallback.
///
/// Priority order:
/// 1. Settings value (if set and non-empty)
/// 2. Environment variable (first match from list)
/// 3. Default value
pub fn get_with_env_fallback(
    setting: &Option<String>,
    env_vars: &[&str],
    default: Option<String>,
) -> Option<String> {
    if let Some(v) = setting {
        if !v.is_empty() {
            return Some(v.clone());
        }
    }

    for env_var in env_vars {
        if let Ok(v) = std::env::var(env_var) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }

    default
}

/// Process-wide proxy env vars owned by [`apply_proxy_env`]. Both the
/// upper- and lower-case spellings are managed so a rogue inherited value in
/// either casing can't leak into in-process HTTP clients.
const PROXY_ENV_VARS: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "http_proxy",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
];

/// Apply proxy settings as **process-wide** environment variables.
///
/// The platform `Settings → Network → Proxy` value is the single source of
/// truth for in-process HTTP clients (LLM API calls, web fetch, telemetry)
/// that read proxy env vars — the same principle the per-subprocess install
/// helpers enforce (see
/// `docs/design/2026-06-10-unified-install-proxy-takeover.md`).
///
/// - Proxy configured → set `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` (+ lower
///   case) and `NO_PROXY`.
/// - Proxy unset → **remove** all of the above so a stale proxy the process
///   inherited from the host shell (e.g. a dead `socks5://127.0.0.1:6153`)
///   can't hijack in-process HTTP. "Not configured" means "direct".
///
/// Should be called early in app startup, after settings are loaded but
/// before any HTTP clients are created.
pub fn apply_proxy_env(settings: &GolishSettings) {
    let proxy = settings
        .network
        .proxy_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match proxy {
        Some(proxy_url) => {
            for var in PROXY_ENV_VARS {
                std::env::set_var(var, proxy_url);
            }
            tracing::info!("Proxy configured process-wide from platform settings");
        }
        None => {
            for var in PROXY_ENV_VARS {
                std::env::remove_var(var);
            }
            tracing::info!(
                "No platform proxy set; cleared inherited proxy env (process-wide direct)"
            );
        }
    }

    let no_proxy = settings
        .network
        .no_proxy
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match no_proxy {
        Some(list) => {
            std::env::set_var("NO_PROXY", list);
            std::env::set_var("no_proxy", list);
        }
        None => {
            std::env::remove_var("NO_PROXY");
            std::env::remove_var("no_proxy");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_env_ref_dollar_format() {
        std::env::set_var("TEST_VAR_1", "test_value_1");

        assert_eq!(
            resolve_env_ref("$TEST_VAR_1"),
            Some("test_value_1".to_string())
        );

        std::env::remove_var("TEST_VAR_1");
    }

    #[test]
    fn test_resolve_env_ref_braces_format() {
        std::env::set_var("TEST_VAR_2", "test_value_2");

        assert_eq!(
            resolve_env_ref("${TEST_VAR_2}"),
            Some("test_value_2".to_string())
        );

        std::env::remove_var("TEST_VAR_2");
    }

    #[test]
    fn test_resolve_env_ref_no_match() {
        assert_eq!(resolve_env_ref("regular_value"), None);
        assert_eq!(resolve_env_ref("$NONEXISTENT_VAR_XYZ_12345"), None);
    }

    #[test]
    fn test_get_with_env_fallback_from_setting() {
        let setting = Some("from_settings".to_string());
        let result = get_with_env_fallback(&setting, &["SOME_VAR"], None);
        assert_eq!(result, Some("from_settings".to_string()));
    }

    #[test]
    fn test_get_with_env_fallback_from_env() {
        std::env::set_var("FALLBACK_TEST_VAR", "from_env");

        let setting = None;
        let result = get_with_env_fallback(&setting, &["FALLBACK_TEST_VAR"], None);
        assert_eq!(result, Some("from_env".to_string()));

        std::env::remove_var("FALLBACK_TEST_VAR");
    }

    #[test]
    fn test_get_with_env_fallback_default() {
        let setting = None;
        let result = get_with_env_fallback(
            &setting,
            &["NONEXISTENT_VAR_ABC"],
            Some("default_value".to_string()),
        );
        assert_eq!(result, Some("default_value".to_string()));
    }

    #[test]
    fn test_get_with_env_fallback_empty_setting() {
        std::env::set_var("EMPTY_SETTING_TEST", "from_env");

        let setting = Some("".to_string());
        let result = get_with_env_fallback(&setting, &["EMPTY_SETTING_TEST"], None);
        assert_eq!(result, Some("from_env".to_string()));

        std::env::remove_var("EMPTY_SETTING_TEST");
    }

    // NOTE: these mutate process-wide env vars. Safe under `cargo nextest`
    // (one process per test); each test also cleans up after itself.
    const PROXY_VARS: &[&str] = &[
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "http_proxy",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ];

    fn clear_proxy_vars() {
        for v in PROXY_VARS.iter().chain(&["NO_PROXY", "no_proxy"]) {
            std::env::remove_var(v);
        }
    }

    #[test]
    fn apply_proxy_env_sets_all_vars_when_configured() {
        clear_proxy_vars();
        let mut settings = GolishSettings::default();
        settings.network.proxy_url = Some("http://127.0.0.1:7890".to_string());
        apply_proxy_env(&settings);
        for v in PROXY_VARS {
            assert_eq!(
                std::env::var(v).ok(),
                Some("http://127.0.0.1:7890".to_string()),
                "{v} should be set process-wide"
            );
        }
        clear_proxy_vars();
    }

    #[test]
    fn apply_proxy_env_clears_inherited_proxy_when_unset() {
        // Simulate a rogue inherited proxy (e.g. a dead Surge socks5).
        for v in PROXY_VARS {
            std::env::set_var(v, "socks5://127.0.0.1:6153");
        }
        let settings = GolishSettings::default(); // proxy_url = None
        apply_proxy_env(&settings);
        for v in PROXY_VARS {
            assert_eq!(
                std::env::var(v).ok(),
                None,
                "{v} should be cleared to force direct"
            );
        }
        clear_proxy_vars();
    }

    #[test]
    fn apply_proxy_env_treats_blank_proxy_as_unset() {
        std::env::set_var("HTTP_PROXY", "socks5://127.0.0.1:6153");
        let mut settings = GolishSettings::default();
        settings.network.proxy_url = Some("   ".to_string());
        apply_proxy_env(&settings);
        assert_eq!(std::env::var("HTTP_PROXY").ok(), None);
        clear_proxy_vars();
    }

    #[test]
    fn apply_proxy_env_clears_no_proxy_when_unset() {
        std::env::set_var("NO_PROXY", "example.com");
        std::env::set_var("no_proxy", "example.com");
        let settings = GolishSettings::default();
        apply_proxy_env(&settings);
        assert_eq!(std::env::var("NO_PROXY").ok(), None);
        assert_eq!(std::env::var("no_proxy").ok(), None);
        clear_proxy_vars();
    }
}
