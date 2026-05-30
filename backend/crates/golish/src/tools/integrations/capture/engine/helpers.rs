//! Low-level capture helpers: webview JavaScript value evaluation
//! (`eval_js_value` / `parse_js_value_title`), cookie access
//! (`fetch_domain_cookies` / `cookie_domain_matches` / `format_joined_cookies`),
//! and the storage-backend persistence bridge (`persist_captured_values`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose, Engine};
use tokio::sync::oneshot;
use uuid::Uuid;

use super::*;

pub(crate) async fn eval_js_value(
    win: &tauri::WebviewWindow,
    expression: &str,
    timeout_ms: u32,
) -> Result<String, String> {
    use tauri::Manager;

    let nonce = Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    let app = win.app_handle();
    let engine = app.state::<Arc<CaptureEngine>>();
    engine
        .js_value_waiters
        .write()
        .await
        .insert(nonce.clone(), tx);

    let nonce_js = serde_json::to_string(&nonce).map_err(|e| e.to_string())?;
    let prefix_js = serde_json::to_string(JS_VALUE_TITLE_PREFIX).map_err(|e| e.to_string())?;
    let script = format!(
        r#"
        (async () => {{
          const __nonce = {nonce_js};
          const __prefix = {prefix_js};
          const __send = (status, value) => {{
            const text = value == null ? "" : String(value);
            const b64 = btoa(unescape(encodeURIComponent(text)));
            document.title = `${{__prefix}}${{__nonce}}:${{status}}:${{b64}}`;
          }};
          try {{
            const value = await (async () => {{ return {expression}; }})();
            if (value == null || String(value) === "") {{
              __send("err", "value was empty");
            }} else {{
              __send("ok", value);
            }}
          }} catch (err) {{
            __send("err", err && err.message ? err.message : String(err));
          }}
        }})();
        "#
    );

    if let Err(e) = win.eval(script) {
        engine.js_value_waiters.write().await.remove(&nonce);
        return Err(format!("eval failed: {e}"));
    }

    match tokio::time::timeout(Duration::from_millis(timeout_ms as u64), rx).await {
        Ok(Ok(value)) => value,
        Ok(Err(_)) => Err("JavaScript value channel closed".to_string()),
        Err(_) => {
            engine.js_value_waiters.write().await.remove(&nonce);
            Err("timed out waiting for JavaScript value".to_string())
        }
    }
}

pub(crate) fn parse_js_value_title(title: &str) -> Option<(String, Result<String, String>)> {
    let rest = title.strip_prefix(JS_VALUE_TITLE_PREFIX)?;
    let mut parts = rest.splitn(3, ':');
    let nonce = parts.next()?.to_string();
    let status = parts.next()?;
    let b64 = parts.next()?;
    let decoded = general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| e.to_string());
    let text = decoded.and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()));
    Some((
        nonce,
        match status {
            "ok" => text,
            "err" => Err(text.unwrap_or_else(|e| e)),
            other => Err(format!("unknown JS value status '{other}'")),
        },
    ))
}

/// Fetch every cookie scoped to `domain` (or any matching parent /
/// child suffix) from the live webview. Routes through
/// `spawn_blocking` because Tauri's cookie API is sync and can
/// deadlock when called from an event handler on Windows.
///
/// **Why we don't use `cookies_for_url`**: wry 0.54's `cookies_for_url`
/// filters cookies with `cookie.domain() == url.domain()` (string
/// equality). `NSHTTPCookie.domain` for a cross-subdomain SSO cookie
/// like `BDUSS` is `".baidu.com"` (leading dot), but `url.domain()`
/// for `https://baidu.com/` is `"baidu.com"` (no dot) — so every
/// `.baidu.com` cookie is silently dropped. We bypass that by fetching
/// the full cookie list via `win.cookies()` and applying RFC 6265
/// domain-match semantics ourselves.
///
/// Per-tool/group isolation (`data_store_identifier` on macOS,
/// `data_directory` on Linux/Windows) means `cookies()` only returns
/// cookies produced for that capture profile, so the broader scope
/// doesn't leak credentials into the main Golish window.
///
/// Returns `Vec<(name, value)>` so callers can `.into_iter().find(...)`
/// without depending on Tauri's `Cookie` type (also keeps the joined
/// formatter unit-testable).
pub(crate) async fn fetch_domain_cookies(
    win: &tauri::WebviewWindow,
    domain: &str,
) -> Result<Vec<(String, String)>, String> {
    let target_host = domain.trim_start_matches('.').to_ascii_lowercase();
    if target_host.is_empty() {
        return Err(format!("capture rule has empty cookie domain ('{domain}')"));
    }
    let webview_current_url = win.url().ok().map(|u| u.to_string());
    let win_clone = win.clone();
    let cookies = tokio::task::spawn_blocking(move || win_clone.cookies())
        .await
        .map_err(|e| format!("spawn_blocking join failed: {e}"))?
        .map_err(|e| format!("cookies() failed: {e}"))?;
    let raw_count = cookies.len();
    let raw_domains: Vec<String> = cookies
        .iter()
        .map(|c| c.domain().unwrap_or("").to_string())
        .collect();
    let pairs: Vec<(String, String)> = cookies
        .into_iter()
        .filter(|c| cookie_domain_matches(c.domain().unwrap_or(""), &target_host))
        .map(|c| (c.name().to_string(), c.value().to_string()))
        .collect();

    // Diagnostic logging: emit cookie NAME list (NEVER value) plus
    // raw_count so we can distinguish "store actually empty" from
    // "everything filtered out". Once production runs consistently
    // show `BDUSS` etc. it's safe to drop this log to `debug!`.
    let names: Vec<&str> = pairs.iter().map(|(n, _)| n.as_str()).collect();
    tracing::info!(
        domain = %domain,
        target_host = %target_host,
        webview_url = ?webview_current_url,
        raw_count,
        raw_domains = ?raw_domains,
        cookie_count = pairs.len(),
        cookie_names = ?names,
        "capture: fetched cookies for domain (names only, values redacted)"
    );

    Ok(pairs)
}

/// RFC 6265 §5.1.3 domain-match semantics, slightly loosened to also
/// accept the cookie's domain attribute when wry hands us back the
/// leading-dot form (`".baidu.com"`).
///
/// Returns `true` when the cookie can legitimately be sent to a request
/// targeting `target_host`. Match cases:
/// - `cookie_domain` (with leading dot stripped, lowercased) equals
///   `target_host`
/// - `cookie_domain` is a sub-domain of `target_host`
///   (e.g. cookie domain `aiqicha.baidu.com` for target `baidu.com`).
///   Strictly speaking RFC 6265 only allows the reverse direction
///   (parent-domain cookie sent to child), but for capture-time
///   "give me every credential the user has on this site" semantics
///   we want both: a `.baidu.com` cookie for an `aiqicha.baidu.com`
///   target AND any host-only `aiqicha.baidu.com` cookie when the
///   capture rule was scoped to `.baidu.com`.
pub(crate) fn cookie_domain_matches(cookie_domain: &str, target_host: &str) -> bool {
    if target_host.is_empty() {
        return false;
    }
    let normalized = cookie_domain
        .trim_start_matches('.')
        .trim()
        .to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    normalized == target_host || normalized.ends_with(&format!(".{target_host}"))
}

/// Filter + format cookies into the `name=value` parts the
/// `CookieJoined` rule will `sep`-join.
///
/// `names` empty → take every cookie (typical ENScan-style "send the
/// full Cookie header" use). Otherwise only cookies whose name
/// appears in `names`. `fmt` supports `{name}` and `{value}`
/// substitution (defaults to `{name}={value}`).
pub(crate) fn format_joined_cookies(
    names: &[String],
    fmt: &str,
    cookies: &[(String, String)],
) -> Vec<String> {
    let want_all = names.is_empty();
    cookies
        .iter()
        .filter(|(n, _)| want_all || names.iter().any(|w| w == n))
        .map(|(n, v)| fmt.replace("{name}", n).replace("{value}", v))
        .collect()
}

/// Persist the captured values via the same `StorageBackend::write`
/// pipeline that `integrations_set` uses (4 steps: get_schema →
/// pool_ready → pick_backend → write).
///
/// Capture engine cannot call `integrations_set` directly (no `invoke`
/// from inside Rust), so we duplicate the sequence here. Keeping this
/// in the engine module instead of `state.rs` is intentional — it
/// isolates the only place we call `DbState::pool_ready` from a
/// non-IPC path.
pub(crate) async fn persist_captured_values(
    app: &tauri::AppHandle,
    tool_id: &str,
    group_id: &str,
    fields: HashMap<String, String>,
) -> Result<(), String> {
    use golish_integrations::SchemaResolver;
    use tauri::Manager;
    let integrations = app.state::<crate::tools::integrations::IntegrationsState>();
    let db = app.state::<crate::state::DbState>();
    let resolved = integrations
        .resolver()
        .get(tool_id)
        .await
        .map_err(|e| format!("get_schema: {e}"))?;
    let pool = db
        .pool_ready()
        .await
        .map_err(|e| format!("pool_ready: {e}"))?
        .clone();
    let backend = integrations
        .pick_backend(&resolved.schema, pool)
        .map_err(|e| format!("pick_backend: {e}"))?;
    backend
        .write(tool_id, group_id, &resolved.schema, fields)
        .await
        .map_err(|e| format!("backend.write: {e}"))?;
    Ok(())
}
