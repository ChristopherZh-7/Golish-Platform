//! The extraction phase: [`CaptureEngine::try_extract`] runs the recipe's
//! rules against the live webview, the navigation handler that fires it on a
//! `success_url_pattern` match, and the per-rule evaluation (`extract_one`)
//! plus the soft-retry failure-reason helpers.

use std::collections::HashMap;
use std::sync::Arc;

use golish_integrations::error::IntegrationResult;
use golish_integrations::schema::{CaptureRecipe, CaptureRule};
use golish_integrations::types::{CaptureState, FailedRule};

use super::*;

impl CaptureEngine {
    /// Run the recipe's extraction rules against the live webview,
    /// persist captured values through the existing storage backend
    /// chain, and emit final state via `integration-capture`.
    ///
    /// Idempotency: once the session is `Extracting` or terminal,
    /// subsequent calls are no-ops. Required-rule failure aborts the
    /// rest of the recipe and marks the session `Failed`.
    pub async fn try_extract(
        &self,
        app: &tauri::AppHandle,
        session_id: &str,
    ) -> IntegrationResult<()> {
        use tauri::Manager;

        let handle = self.get(session_id).await?;

        // Idempotency guard: don't re-enter if already extracting /
        // terminal. Drop the read guard before async work continues.
        let (rules, tool_id, group_id) = {
            let s = handle.inner.read().await;
            if s.state.is_terminal() || s.state == CaptureState::Extracting {
                return Ok(());
            }
            (
                s.recipe.rules.clone(),
                s.tool_id.clone(),
                s.group_id.clone(),
            )
        };
        self.transition_and_emit(app, session_id, CaptureState::Extracting, None)
            .await?;

        // Find the webview we opened. If the user closed it manually
        // between `success_url_pattern` match and now, treat as Failed.
        let label = format!("capture-{}", session_id);
        let win = match app.get_webview_window(&label) {
            Some(w) => w,
            None => {
                self.transition_and_emit(
                    app,
                    session_id,
                    CaptureState::Failed,
                    Some("[WEBVIEW_CREATE_FAILED] webview window vanished mid-extract".to_string()),
                )
                .await?;
                return Ok(());
            }
        };

        let mut captured: HashMap<String, String> = HashMap::new();
        let mut failed: Vec<FailedRule> = Vec::new();
        let mut required_failure: Option<(usize, String)> = None;

        for (idx, rule) in rules.iter().enumerate() {
            match extract_one(&win, rule).await {
                Ok((target_field, value)) => {
                    captured.insert(target_field, value);
                }
                Err(reason) => {
                    let is_required = rule_is_required(rule);
                    failed.push(FailedRule {
                        rule_index: idx,
                        reason: reason.clone(),
                    });
                    if is_required {
                        required_failure = Some((idx, reason));
                        break;
                    }
                }
            }
        }

        // Stash captured + failed details onto the session before the
        // terminal transition so the event payload carries them.
        let captured_field_names: Vec<String> = captured.keys().cloned().collect();
        {
            let mut s = handle.inner.write().await;
            s.captured_fields = captured_field_names.clone();
            s.failed_rules = failed.clone();
        }

        // Required failure short-circuit — UNLESS the rule asked us
        // to soft-retry on the next navigation (e.g. CookieJoined
        // declared `required_names` but the cookie jar doesn't have
        // them yet because the user is still mid-login). In that case
        // the webview stays open and the navigation handler will fire
        // try_extract again the next time the URL changes.
        if let Some((idx, reason)) = required_failure {
            if reason.starts_with("[SOFT_RETRY]") {
                tracing::info!(
                    session_id = %session_id,
                    rule_index = idx,
                    reason = %reason,
                    "capture: required rule signalled soft-retry; webview stays open"
                );
                self.rearm_after_soft_retry(&handle).await;
                Self::spawn_soft_retry_probe(app.clone(), session_id.to_string());
                return Ok(());
            }
            self.transition_and_emit(
                app,
                session_id,
                CaptureState::Failed,
                Some(format!(
                    "[CAPTURE_RULE_FAILED] required rule #{idx}: {reason}"
                )),
            )
            .await?;
            let _ = win.close();
            return Ok(());
        }

        // Persist captured values via the same backend chain used by
        // `integrations_set`. We don't call the IPC command directly
        // (no access to invoke from here), so we duplicate the 4-step
        // sequence: get_schema → pool_ready → pick_backend → write.
        if !captured.is_empty() {
            if let Err(e) =
                persist_captured_values(app, &tool_id, &group_id, captured.clone()).await
            {
                self.transition_and_emit(
                    app,
                    session_id,
                    CaptureState::Failed,
                    Some(format!("[STORAGE_WRITE_FAILED] {e}")),
                )
                .await?;
                let _ = win.close();
                return Ok(());
            }
        }

        // Success / partial.
        let next = if failed.is_empty() {
            CaptureState::Captured
        } else {
            CaptureState::Partial
        };
        self.transition_and_emit(app, session_id, next, None)
            .await?;
        let _ = win.close();
        Ok(())
    }
}

/// Whether a rule's `required` flag is set. P1 MVP: required=true
/// rules abort the whole capture; required=false rules degrade to
/// Partial state.
fn rule_is_required(rule: &CaptureRule) -> bool {
    match rule {
        CaptureRule::Cookie { required, .. }
        | CaptureRule::CookieJoined { required, .. }
        | CaptureRule::LocalStorage { required, .. }
        | CaptureRule::SessionStorage { required, .. }
        | CaptureRule::PageContent { required, .. }
        | CaptureRule::UrlQuery { required, .. }
        | CaptureRule::RequestHeader { required, .. } => *required,
    }
}

/// Navigation event handler. Wired by [`CaptureEngine::start_webview`].
///
/// Tauri 2 invokes this `Fn(&Url) -> bool` synchronously from the
/// platform's webview thread. We immediately spawn an async task and
/// return so we never block navigation. The async task does the
/// regex match and (if matched) fires extraction.
pub(crate) async fn on_navigation_event(
    app: &tauri::AppHandle,
    session_id: &str,
    recipe: &CaptureRecipe,
    new_url: &str,
) {
    use tauri::Manager;
    let Some(pat) = recipe.success_url_pattern.as_ref() else {
        return;
    };
    let re = match regex::Regex::new(pat) {
        Ok(re) => re,
        Err(e) => {
            tracing::warn!(
                session_id = %session_id,
                pattern = %pat,
                error = %e,
                "capture: invalid success_url_pattern (schema validation gap)"
            );
            return;
        }
    };
    if !re.is_match(new_url) {
        return;
    }
    tracing::info!(
        session_id = %session_id,
        url = %new_url,
        "capture: success_url_pattern matched; triggering extraction"
    );
    let engine = app.state::<Arc<CaptureEngine>>();
    let engine = engine.inner().clone();
    if let Err(e) = engine.try_extract(app, session_id).await {
        tracing::error!(
            session_id = %session_id,
            error = %e,
            "capture: try_extract failed"
        );
    }
}

/// Run one extraction rule against the live webview. Returns
/// `(target_field, value)` on success or `Err(reason)` otherwise.
async fn extract_one(
    win: &tauri::WebviewWindow,
    rule: &CaptureRule,
) -> Result<(String, String), String> {
    match rule {
        CaptureRule::Cookie {
            domain,
            name,
            target_field,
            required,
        } => {
            let cookies = fetch_domain_cookies(win, domain).await?;
            let value = cookies
                .into_iter()
                .find(|(n, _)| n == name)
                .ok_or_else(|| cookie_failure_reason(domain, name, *required))?
                .1;
            Ok((target_field.clone(), value))
        }

        CaptureRule::CookieJoined {
            domain,
            names,
            sep,
            fmt,
            target_field,
            required_names,
            required,
            min_count,
        } => {
            // `names = []` means "all cookies belonging to this domain
            // (or any subdomain) — wanted format = full Cookie header".
            // For sites with strong session-binding like aiqicha.baidu.com
            // (BDUSS alone trips the safety wall — needs BAIDUID + PSTM
            // + H_PS_PSSID + 等 一起) the user typically wants every
            // baidu.com-domain cookie.
            let cookies = fetch_domain_cookies(win, domain).await?;

            // Login-state proof check: when the rule declares
            // `required_names`, every one of those cookies must be
            // present in the live jar before we count this navigation
            // as "login succeeded". Used to keep the loose AQC
            // `success_url_pattern` (which must also fire on the root
            // path the user is redirected back to after two-factor
            // verification) from latching onto the pre-login load
            // and persisting an anonymous Cookie header. Returning Err
            // here is treated as a soft retry by the caller — capture
            // engine simply waits for the next navigation.
            if !required_names.is_empty() {
                let have: std::collections::HashSet<&str> =
                    cookies.iter().map(|(n, _)| n.as_str()).collect();
                let missing: Vec<&String> = required_names
                    .iter()
                    .filter(|n| !have.contains(n.as_str()))
                    .collect();
                if !missing.is_empty() {
                    // Prefixed `[SOFT_RETRY]` so the caller (try_extract)
                    // re-arms instead of marking the whole session
                    // Failed — the webview stays open for the user to
                    // finish login on the next navigation.
                    return Err(format!(
                        "[SOFT_RETRY] required cookies not yet present on '{domain}' (missing {missing:?}) — \
                         likely still mid-login, waiting for next navigation"
                    ));
                }
            }

            let parts = format_joined_cookies(names, fmt, &cookies);
            if *min_count > 0 && parts.len() < *min_count {
                return Err(cookie_joined_min_count_failure_reason(
                    domain,
                    parts.len(),
                    *min_count,
                    *required,
                ));
            }
            if parts.is_empty() {
                return Err(cookie_joined_failure_reason(domain, names, *required));
            }
            let value = parts.join(sep);
            Ok((target_field.clone(), value))
        }

        CaptureRule::LocalStorage {
            key, target_field, ..
        } => {
            let key_js = serde_json::to_string(key).map_err(|e| e.to_string())?;
            let value = eval_js_value(win, &format!("window.localStorage.getItem({key_js})"), 3000)
                .await
                .map_err(|e| format!("localStorage key '{key}' not found: {e}"))?;
            Ok((target_field.clone(), value))
        }

        CaptureRule::SessionStorage {
            key, target_field, ..
        } => {
            let key_js = serde_json::to_string(key).map_err(|e| e.to_string())?;
            let value = eval_js_value(
                win,
                &format!("window.sessionStorage.getItem({key_js})"),
                3000,
            )
            .await
            .map_err(|e| format!("sessionStorage key '{key}' not found: {e}"))?;
            Ok((target_field.clone(), value))
        }

        CaptureRule::PageContent {
            selector,
            attribute,
            wait_ms,
            target_field,
            ..
        } => {
            let selector_js = serde_json::to_string(selector).map_err(|e| e.to_string())?;
            let attribute_js = match attribute {
                Some(attr) => serde_json::to_string(attr).map_err(|e| e.to_string())?,
                None => "null".to_string(),
            };
            let wait_ms = (*wait_ms).max(100);
            let expression = format!(
                r#"
                (async () => {{
                  const selector = {selector_js};
                  const attribute = {attribute_js};
                  const deadline = Date.now() + {wait_ms};
                  while (Date.now() <= deadline) {{
                    const el = document.querySelector(selector);
                    if (el) {{
                      return attribute ? el.getAttribute(attribute) : (el.textContent || "");
                    }}
                    await new Promise((resolve) => setTimeout(resolve, 100));
                  }}
                  throw new Error(`selector not found: ${{selector}}`);
                }})()
                "#
            );
            let value = eval_js_value(win, &expression, wait_ms + 1000).await?;
            Ok((target_field.clone(), value))
        }

        CaptureRule::UrlQuery {
            name, target_field, ..
        } => {
            let url = win
                .url()
                .map_err(|e| format!("read current URL failed: {e}"))?;
            let value = url
                .query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
                .ok_or_else(|| format!("query parameter '{name}' not found in {url}"))?;
            Ok((target_field.clone(), value))
        }

        CaptureRule::RequestHeader {
            name,
            url_pattern,
            target_field,
            required,
        } => {
            let name_js =
                serde_json::to_string(&name.to_ascii_lowercase()).map_err(|e| e.to_string())?;
            let pattern_js = match url_pattern {
                Some(pattern) => serde_json::to_string(pattern).map_err(|e| e.to_string())?,
                None => "null".to_string(),
            };
            let expression = format!(
                r#"
                (() => {{
                  const headerName = {name_js};
                  const patternText = {pattern_js};
                  const pattern = patternText ? new RegExp(patternText) : null;
                  const records = window.__GOLISH_CAPTURE_REQUESTS__ || [];
                  for (let i = records.length - 1; i >= 0; i--) {{
                    const record = records[i] || {{}};
                    if (pattern && !pattern.test(String(record.url || ""))) continue;
                    const headers = record.headers || {{}};
                    if (headers[headerName]) return headers[headerName];
                    for (const key of Object.keys(headers)) {{
                      if (String(key).toLowerCase() === headerName) return headers[key];
                    }}
                  }}
                  return null;
                }})()
                "#
            );
            let value = eval_js_value(win, &expression, 3000)
                .await
                .map_err(|e| request_header_failure_reason(name, &e, *required))?;
            Ok((target_field.clone(), value))
        }
    }
}

pub(crate) fn request_header_failure_reason(name: &str, reason: &str, required: bool) -> String {
    let message = format!("request header '{name}' not observed: {reason}");
    if required && reason == "value was empty" {
        format!(
            "[SOFT_RETRY] {message} — likely still mid-login or no matching API request yet, waiting for next navigation"
        )
    } else {
        message
    }
}

pub(crate) fn cookie_failure_reason(domain: &str, name: &str, required: bool) -> String {
    let message = format!("cookie '{name}' not found in domain '{domain}'");
    if required {
        format!("[SOFT_RETRY] {message} — likely still mid-login, waiting for next navigation")
    } else {
        message
    }
}

pub(crate) fn cookie_joined_failure_reason(
    domain: &str,
    names: &[String],
    required: bool,
) -> String {
    let want_summary = if names.is_empty() {
        "<all>".to_string()
    } else {
        format!("{names:?}")
    };
    let message = format!("no cookies matched for domain '{domain}' (wanted {want_summary})");
    if required {
        format!("[SOFT_RETRY] {message} — likely still mid-login, waiting for next navigation")
    } else {
        message
    }
}

pub(crate) fn cookie_joined_min_count_failure_reason(
    domain: &str,
    actual: usize,
    expected: usize,
    required: bool,
) -> String {
    let message = format!(
        "not enough cookies matched for domain '{domain}' (got {actual}, need at least {expected})"
    );
    if required {
        format!("[SOFT_RETRY] {message} — likely still mid-login, waiting for next navigation")
    } else {
        message
    }
}
