//! `CaptureEngine` — owns the in-flight session registry and drives
//! the capture state machine.
//!
//! ## Concurrency model
//!
//! Two layers of locking:
//!   1. `sessions: RwLock<HashMap<String, CaptureSessionHandle>>` —
//!      registry-level. Held just long enough to insert / lookup /
//!      remove. Concurrent reads of different sessions don't contend.
//!   2. Each `CaptureSessionHandle` owns its own `RwLock<CaptureSession>`
//!      — held during per-session mutation (state transition / field
//!      capture / error stash).
//!
//! ## State machine
//!
//! All non-terminal → terminal transitions go through
//! [`CaptureEngine::transition`]. Once terminal, subsequent transition
//! calls are silently ignored so a stray TTL tick can't clobber an
//! already-Captured session. Terminal transitions also trigger
//! [`data_dir::cleanup_session_dir`].
//!
//! T2.3 — T2.6 add: webview construction (`start_webview`),
//! navigation handler that fires extraction on `success_url_pattern`
//! match, cookie extraction (`try_extract`), TTL watcher, and event
//! emission on the `"integration-capture"` Tauri channel.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose, Engine};
use golish_integrations::error::{IntegrationError, IntegrationResult};
use golish_integrations::schema::{CaptureRecipe, CaptureRule};
use golish_integrations::types::{CaptureEventPayload, CaptureState, FailedRule};
use tokio::sync::{oneshot, RwLock};
use uuid::Uuid;

use super::data_dir;
use super::session::{CaptureSession, CaptureSessionHandle};

/// Garbage-collection threshold for terminal sessions: anything older
/// than 1 hour gets dropped from the registry. The frontend keeps its
/// own snapshot via the `_start` response so a missed status poll
/// after GC merely returns `CAPTURE_SESSION_NOT_FOUND` (handled by
/// the IPC layer as a 404).
const GC_RETENTION_MS: i64 = 3600 * 1000;
const JS_VALUE_TITLE_PREFIX: &str = "__GOLISH_CAPTURE_VALUE__:";
const CAPTURE_REQUEST_INIT_SCRIPT: &str = r#"
(() => {
  if (window.__GOLISH_CAPTURE_REQUEST_MONITOR__) return;
  Object.defineProperty(window, "__GOLISH_CAPTURE_REQUEST_MONITOR__", { value: true });
  const records = [];
  Object.defineProperty(window, "__GOLISH_CAPTURE_REQUESTS__", { value: records });
  const toHeaders = (input) => {
    const out = {};
    if (!input) return out;
    try {
      if (input instanceof Headers) {
        input.forEach((value, key) => { out[String(key).toLowerCase()] = String(value); });
      } else if (Array.isArray(input)) {
        for (const pair of input) {
          if (pair && pair.length >= 2) out[String(pair[0]).toLowerCase()] = String(pair[1]);
        }
      } else if (typeof input === "object") {
        for (const key of Object.keys(input)) out[String(key).toLowerCase()] = String(input[key]);
      }
    } catch (_) {}
    return out;
  };
  const remember = (url, headers) => {
    records.push({ url: String(url || window.location.href), headers: headers || {}, at: Date.now() });
    if (records.length > 200) records.splice(0, records.length - 200);
  };
  if (typeof window.fetch === "function") {
    const originalFetch = window.fetch;
    window.fetch = function(input, init) {
      const url = typeof input === "string" ? input : (input && input.url) || window.location.href;
      remember(url, { ...toHeaders(input && input.headers), ...toHeaders(init && init.headers) });
      return originalFetch.apply(this, arguments);
    };
  }
  if (window.XMLHttpRequest) {
    const proto = window.XMLHttpRequest.prototype;
    const originalOpen = proto.open;
    const originalSetRequestHeader = proto.setRequestHeader;
    const originalSend = proto.send;
    proto.open = function(method, url) {
      this.__golishCaptureUrl = url;
      this.__golishCaptureHeaders = {};
      return originalOpen.apply(this, arguments);
    };
    proto.setRequestHeader = function(name, value) {
      this.__golishCaptureHeaders[String(name).toLowerCase()] = String(value);
      return originalSetRequestHeader.apply(this, arguments);
    };
    proto.send = function() {
      remember(this.__golishCaptureUrl || window.location.href, this.__golishCaptureHeaders || {});
      return originalSend.apply(this, arguments);
    };
  }
})();
"#;

/// Tauri-managed singleton (`tauri::State<Arc<CaptureEngine>>`).
///
/// Sessions are keyed by their UUID-v4 `session_id`. The registry is
/// never persisted — a Golish restart wipes any in-flight sessions
/// (which is correct: their webviews are gone anyway).
pub struct CaptureEngine {
    sessions: RwLock<HashMap<String, CaptureSessionHandle>>,
    js_value_waiters: RwLock<HashMap<String, oneshot::Sender<Result<String, String>>>>,
}

impl Default for CaptureEngine {
    fn default() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            js_value_waiters: RwLock::new(HashMap::new()),
        }
    }
}

impl CaptureEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new session. Returns the handle (caller is
    /// responsible for actually creating the webview via
    /// [`Self::start_webview`] — separated so registration can be
    /// unit-tested without Tauri).
    ///
    /// Rejects with `CaptureAlreadyRunning` when any non-terminal
    /// session exists for the same `(tool_id, group_id)` pair.
    pub async fn register(
        &self,
        tool_id: String,
        group_id: String,
        recipe: CaptureRecipe,
    ) -> IntegrationResult<CaptureSessionHandle> {
        {
            let map = self.sessions.read().await;
            for h in map.values() {
                let s = h.inner.read().await;
                if !s.state.is_terminal() && s.tool_id == tool_id && s.group_id == group_id {
                    return Err(IntegrationError::CaptureAlreadyRunning { tool_id, group_id });
                }
            }
        }
        let sid = Uuid::new_v4().to_string();
        let session = CaptureSession::new(sid.clone(), tool_id, group_id, recipe);
        let handle = CaptureSessionHandle::new(session);
        self.sessions
            .write()
            .await
            .insert(sid.clone(), handle.clone());
        Ok(handle)
    }

    /// Look up a session by id. Returns `CaptureSessionNotFound` when
    /// the id was never registered or was GC'd.
    pub async fn get(&self, session_id: &str) -> IntegrationResult<CaptureSessionHandle> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| IntegrationError::CaptureSessionNotFound(session_id.to_string()))
    }

    /// Transition a session into a new state.
    ///
    /// Once terminal, further calls are no-ops (idempotent). Terminal
    /// transitions trigger [`data_dir::cleanup_session_dir`].
    pub async fn transition(
        &self,
        session_id: &str,
        next: CaptureState,
        error: Option<String>,
    ) -> IntegrationResult<()> {
        let handle = self.get(session_id).await?;
        {
            let mut s = handle.inner.write().await;
            if s.state.is_terminal() {
                tracing::debug!(
                    session_id = %session_id,
                    current = ?s.state,
                    next = ?next,
                    "capture: ignoring transition (already terminal)"
                );
                return Ok(());
            }
            s.transition(next);
            if let Some(msg) = error {
                s.error_message = Some(msg);
            }
        }
        if next.is_terminal() {
            data_dir::cleanup_session_dir(session_id);
        }
        Ok(())
    }

    /// Convenience: mark cancelled (used by the manual "cancel"
    /// command and by webview-close hooks).
    pub async fn cancel(&self, session_id: &str) -> IntegrationResult<()> {
        self.transition(session_id, CaptureState::Cancelled, None)
            .await
    }

    /// Drop terminal sessions older than [`GC_RETENTION_MS`].
    /// Called periodically by the TTL watcher (T2.5).
    pub async fn gc(&self) {
        let cutoff = chrono::Utc::now().timestamp_millis() - GC_RETENTION_MS;
        let to_remove: Vec<String> = {
            let map = self.sessions.read().await;
            let mut v = Vec::new();
            for (sid, h) in map.iter() {
                let s = h.inner.read().await;
                if s.state.is_terminal() && s.updated_at_ms < cutoff {
                    v.push(sid.clone());
                }
            }
            v
        };
        if to_remove.is_empty() {
            return;
        }
        let mut map = self.sessions.write().await;
        for sid in to_remove {
            map.remove(&sid);
            tracing::debug!(session_id = %sid, "capture: gc removed terminal session");
        }
    }

    /// Test-only: returns the number of sessions currently in the
    /// registry. Useful for asserting GC behavior.
    #[cfg(test)]
    pub(crate) async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }

    /// Same as [`Self::transition`] but also emits the
    /// `"integration-capture"` Tauri event for the new state. Use
    /// this from every path that needs the UI to react (cancel /
    /// extract / TTL timeout / failure).
    pub async fn transition_and_emit(
        &self,
        app: &tauri::AppHandle,
        session_id: &str,
        next: CaptureState,
        error: Option<String>,
    ) -> IntegrationResult<()> {
        use tauri::Emitter;
        self.transition(session_id, next, error).await?;
        let payload = {
            let handle = self.get(session_id).await?;
            let s = handle.inner.read().await;
            CaptureEventPayload {
                session_id: s.session_id.clone(),
                tool_id: s.tool_id.clone(),
                group_id: s.group_id.clone(),
                state: s.state,
                captured_fields: s.captured_fields.clone(),
                failed_rules: s.failed_rules.clone(),
                error_message: s.error_message.clone(),
            }
        };
        if let Err(e) = app.emit("integration-capture", payload) {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "capture: failed to emit integration-capture event"
            );
        }
        Ok(())
    }

    /// Re-arm a session after a required rule explicitly asked for a
    /// soft retry. This is the "still mid-login" path: stale extraction
    /// details must be cleared and the state must become retryable again
    /// so the next matching navigation can run `try_extract`.
    async fn rearm_after_soft_retry(&self, handle: &CaptureSessionHandle) {
        let mut s = handle.inner.write().await;
        s.failed_rules.clear();
        s.captured_fields.clear();
        s.error_message = None;
        s.transition(CaptureState::WaitingLogin);
    }

    fn spawn_soft_retry_probe(app: tauri::AppHandle, session_id: String) {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            use tauri::Manager;
            let engine = app.state::<Arc<CaptureEngine>>();
            let engine = engine.inner().clone();
            if let Err(e) = engine.try_extract(&app, &session_id).await {
                tracing::debug!(
                    session_id = %session_id,
                    error = %e,
                    "capture: delayed soft-retry probe could not extract yet"
                );
            }
        });
    }

    async fn deliver_js_value(&self, nonce: &str, value: Result<String, String>) {
        let sender = self.js_value_waiters.write().await.remove(nonce);
        if let Some(sender) = sender {
            let _ = sender.send(value);
        }
    }

    /// Build the Tauri WebviewWindow for an already-registered session
    /// and wire its `on_navigation` callback to fire extraction when
    /// `success_url_pattern` matches.
    ///
    /// The webview's data store is isolated per `(tool_id, group_id)`
    /// (see [`super::webview_isolation::apply_isolation`]). A failure
    /// here returns `WebviewCreateFailed`; the caller is responsible
    /// for transitioning the session into `Failed`.
    pub async fn start_webview(
        &self,
        app: &tauri::AppHandle,
        handle: &CaptureSessionHandle,
    ) -> IntegrationResult<()> {
        use tauri::{WebviewUrl, WebviewWindowBuilder};

        // 1. Snapshot what we need from the session under a short read lock.
        let (sid, tool_id, group_id, login_url, recipe) = {
            let s = handle.inner.read().await;
            (
                s.session_id.clone(),
                s.tool_id.clone(),
                s.group_id.clone(),
                s.recipe.login_url.clone(),
                s.recipe.clone(),
            )
        };

        // 2. Reserve the persistent per-tool/group profile directory.
        //    Linux / Windows use this as `data_directory`; macOS uses
        //    a stable `data_store_identifier` derived from the same key.
        let profile_key = data_dir::profile_key(&tool_id, &group_id);
        let profile_dir = data_dir::profile_dir(&tool_id, &group_id)?;

        // 3. Parse the URL (already schema-validated, but the builder
        //    needs an actual `url::Url`).
        let url = login_url
            .parse::<url::Url>()
            .map_err(|e| IntegrationError::CaptureInvalidUrl(format!("{login_url}: {e}")))?;

        // 4. Build the window. Label is `capture-<session_id>` (unique).
        let label = format!("capture-{}", sid);
        let host_for_title = url.host_str().unwrap_or("?").to_string();

        // Capture state needed by the navigation callback. `Fn` (not
        // FnOnce / FnMut) so we re-clone inside the closure each fire.
        let app_for_cb = app.clone();
        let sid_for_cb = sid.clone();
        let recipe_for_cb = recipe.clone();
        let app_for_title_cb = app.clone();

        let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
            .title(format!("Golish · 凭据抓取: {host_for_title}"))
            .inner_size(900.0, 700.0)
            .center()
            .focused(true)
            .visible(true)
            .initialization_script(CAPTURE_REQUEST_INIT_SCRIPT);
        let builder =
            super::webview_isolation::apply_isolation(builder, &profile_key, &profile_dir);
        let builder = builder.on_document_title_changed(move |_win, title| {
            if let Some((nonce, value)) = parse_js_value_title(&title) {
                let app = app_for_title_cb.clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Manager;
                    let engine = app.state::<Arc<CaptureEngine>>();
                    engine.deliver_js_value(&nonce, value).await;
                });
            }
        });
        let builder = builder.on_navigation(move |new_url: &url::Url| {
            let app = app_for_cb.clone();
            let sid = sid_for_cb.clone();
            let recipe = recipe_for_cb.clone();
            let url_str = new_url.to_string();
            tauri::async_runtime::spawn(async move {
                on_navigation_event(&app, &sid, &recipe, &url_str).await;
            });
            // Returning `true` allows the navigation to proceed; we
            // never block navigations in P1 MVP (users explicitly opt
            // into this flow).
            true
        });

        builder
            .build()
            .map_err(|e| IntegrationError::WebviewCreateFailed(e.to_string()))?;

        Ok(())
    }

    /// Clear the persisted browser login state for one capture profile.
    /// This does not clear the credential already written to the
    /// integration storage backend; it only resets the next ⚡ webview.
    pub async fn clear_profile(
        &self,
        app: &tauri::AppHandle,
        tool_id: &str,
        group_id: &str,
    ) -> IntegrationResult<()> {
        use tauri::{WebviewUrl, WebviewWindowBuilder};

        let profile_key = data_dir::profile_key(tool_id, group_id);
        let profile_dir = data_dir::profile_dir(tool_id, group_id)?;
        let label = format!("capture-clear-{}", Uuid::new_v4());
        let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
            .title("Golish · Clear capture login state")
            .visible(false);
        let builder =
            super::webview_isolation::apply_isolation(builder, &profile_key, &profile_dir);
        let win = builder
            .build()
            .map_err(|e| IntegrationError::WebviewCreateFailed(e.to_string()))?;
        win.clear_all_browsing_data()
            .map_err(|e| IntegrationError::WebviewCreateFailed(e.to_string()))?;
        let _ = win.close();
        data_dir::cleanup_profile_dir(tool_id, group_id);
        Ok(())
    }

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

    /// Spawn a background task that periodically scans for sessions
    /// past their TTL and transitions them to `Timeout`, then runs GC.
    ///
    /// Called once from `app.setup` (see Tauri builder wiring).
    pub fn spawn_ttl_watcher(self: Arc<Self>, app: tauri::AppHandle) {
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                let now_ms = chrono::Utc::now().timestamp_millis();
                let to_timeout: Vec<String> = {
                    let map = self.sessions.read().await;
                    let mut v = Vec::new();
                    for (sid, h) in map.iter() {
                        let s = h.inner.read().await;
                        if s.state.is_terminal() {
                            continue;
                        }
                        let expires = s.started_at_ms + (s.timeout_secs as i64) * 1000;
                        if now_ms > expires {
                            v.push(sid.clone());
                        }
                    }
                    v
                };
                for sid in to_timeout {
                    let _ = self
                        .transition_and_emit(
                            &app,
                            &sid,
                            CaptureState::Timeout,
                            Some(
                                "[CAPTURE_TIMEOUT] capture session expired without completion"
                                    .to_string(),
                            ),
                        )
                        .await;
                    // Best-effort close the lingering webview.
                    use tauri::Manager;
                    let label = format!("capture-{}", sid);
                    if let Some(win) = app.get_webview_window(&label) {
                        let _ = win.close();
                    }
                }
                self.gc().await;
            }
        });
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
async fn on_navigation_event(
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

fn request_header_failure_reason(name: &str, reason: &str, required: bool) -> String {
    let message = format!("request header '{name}' not observed: {reason}");
    if required && reason == "value was empty" {
        format!(
            "[SOFT_RETRY] {message} — likely still mid-login or no matching API request yet, waiting for next navigation"
        )
    } else {
        message
    }
}

fn cookie_failure_reason(domain: &str, name: &str, required: bool) -> String {
    let message = format!("cookie '{name}' not found in domain '{domain}'");
    if required {
        format!("[SOFT_RETRY] {message} — likely still mid-login, waiting for next navigation")
    } else {
        message
    }
}

fn cookie_joined_failure_reason(domain: &str, names: &[String], required: bool) -> String {
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

fn cookie_joined_min_count_failure_reason(
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

async fn eval_js_value(
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

fn parse_js_value_title(title: &str) -> Option<(String, Result<String, String>)> {
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
async fn fetch_domain_cookies(
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
fn cookie_domain_matches(cookie_domain: &str, target_host: &str) -> bool {
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
fn format_joined_cookies(names: &[String], fmt: &str, cookies: &[(String, String)]) -> Vec<String> {
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
/// in `engine.rs` instead of `state.rs` is intentional — it isolates
/// the only place we call `DbState::pool_ready` from a non-IPC path.
async fn persist_captured_values(
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

#[cfg(test)]
mod tests {
    use super::*;
    use golish_integrations::schema::{CaptureRecipe, CaptureRule};
    use golish_integrations::types::CaptureState;

    fn aqc_recipe() -> CaptureRecipe {
        CaptureRecipe {
            login_url: "https://aiqicha.baidu.com".into(),
            success_url_pattern: None,
            visit_url: None,
            instructions: None,
            timeout_secs: 60,
            rules: vec![CaptureRule::Cookie {
                domain: ".aiqicha.baidu.com".into(),
                name: "BDUSS".into(),
                target_field: "cookies.aqc".into(),
                required: true,
            }],
        }
    }

    #[tokio::test]
    async fn register_returns_unique_session_id() {
        let eng = CaptureEngine::new();
        let h1 = eng
            .register("enscan-go".into(), "aqc".into(), aqc_recipe())
            .await
            .unwrap();
        let h2 = eng
            .register("enscan-go".into(), "tyc".into(), aqc_recipe())
            .await
            .unwrap();
        assert_ne!(h1.session_id, h2.session_id);
    }

    #[tokio::test]
    async fn register_rejects_duplicate_tool_group() {
        let eng = CaptureEngine::new();
        let _ = eng
            .register("enscan-go".into(), "aqc".into(), aqc_recipe())
            .await
            .unwrap();
        let err = eng
            .register("enscan-go".into(), "aqc".into(), aqc_recipe())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            IntegrationError::CaptureAlreadyRunning { .. }
        ));
    }

    #[tokio::test]
    async fn register_after_terminal_allows_restart() {
        let eng = CaptureEngine::new();
        let h = eng
            .register("enscan-go".into(), "aqc".into(), aqc_recipe())
            .await
            .unwrap();
        eng.cancel(&h.session_id).await.unwrap();
        let _ = eng
            .register("enscan-go".into(), "aqc".into(), aqc_recipe())
            .await
            .expect("should allow restart after cancel");
    }

    #[tokio::test]
    async fn transition_to_terminal_is_idempotent() {
        let eng = CaptureEngine::new();
        let h = eng
            .register("t".into(), "g".into(), aqc_recipe())
            .await
            .unwrap();
        eng.transition(&h.session_id, CaptureState::Captured, None)
            .await
            .unwrap();
        eng.transition(
            &h.session_id,
            CaptureState::Failed,
            Some("late update".into()),
        )
        .await
        .unwrap();
        let s = h.inner.read().await;
        assert_eq!(s.state, CaptureState::Captured);
        assert!(
            s.error_message.is_none(),
            "error must not be stamped post-terminal"
        );
    }

    #[tokio::test]
    async fn get_unknown_returns_not_found() {
        let eng = CaptureEngine::new();
        let err = eng.get("nope").await.unwrap_err();
        assert!(matches!(err, IntegrationError::CaptureSessionNotFound(_)));
    }

    #[tokio::test]
    async fn cancel_transitions_to_cancelled() {
        let eng = CaptureEngine::new();
        let h = eng
            .register("t".into(), "g".into(), aqc_recipe())
            .await
            .unwrap();
        eng.cancel(&h.session_id).await.unwrap();
        let s = h.inner.read().await;
        assert_eq!(s.state, CaptureState::Cancelled);
        assert!(s.state.is_terminal());
    }

    #[tokio::test]
    async fn soft_retry_rearms_waiting_login_after_empty_cookie_attempt() {
        let eng = CaptureEngine::new();
        let h = eng
            .register("t".into(), "g".into(), aqc_recipe())
            .await
            .unwrap();

        {
            let mut s = h.inner.write().await;
            s.transition(CaptureState::Extracting);
            s.failed_rules.push(FailedRule {
                rule_index: 0,
                reason: "[SOFT_RETRY] required cookies not yet present".into(),
            });
            s.captured_fields.push("cookies.aqc".into());
        }

        eng.rearm_after_soft_retry(&h).await;

        let s = h.inner.read().await;
        assert_eq!(s.state, CaptureState::WaitingLogin);
        assert!(s.failed_rules.is_empty());
        assert!(s.captured_fields.is_empty());
    }

    #[test]
    fn required_request_header_failures_are_soft_retryable() {
        let reason = request_header_failure_reason("X-Tycid", "value was empty", true);

        assert!(
            reason.starts_with("[SOFT_RETRY]"),
            "missing required request headers should keep the capture window open"
        );
        assert!(reason.contains("request header 'X-Tycid' not observed"));
    }

    #[test]
    fn required_cookie_failures_are_soft_retryable() {
        let reason = cookie_failure_reason(".qimai.cn", "QIMOSESSID", true);

        assert!(
            reason.starts_with("[SOFT_RETRY]"),
            "missing required cookies should keep the capture window open"
        );
        assert!(reason.contains("cookie 'QIMOSESSID' not found"));
    }

    #[test]
    fn required_cookie_joined_empty_matches_are_soft_retryable() {
        let reason = cookie_joined_failure_reason(".riskbird.com", &[], true);

        assert!(
            reason.starts_with("[SOFT_RETRY]"),
            "empty required cookie joins should keep the capture window open"
        );
        assert!(reason.contains("no cookies matched for domain '.riskbird.com'"));
    }

    #[test]
    fn required_cookie_joined_min_count_failures_are_soft_retryable() {
        let reason = cookie_joined_min_count_failure_reason(".qimai.cn", 1, 2, true);

        assert!(
            reason.starts_with("[SOFT_RETRY]"),
            "anonymous cookie sets should keep the capture window open"
        );
        assert!(reason.contains("got 1, need at least 2"));
    }

    #[test]
    fn format_joined_cookies_all_when_names_empty() {
        let cookies = vec![
            ("BDUSS".to_string(), "xx".to_string()),
            ("BAIDUID".to_string(), "yy".to_string()),
            ("PSTM".to_string(), "12345".to_string()),
        ];
        let parts = format_joined_cookies(&[], "{name}={value}", &cookies);
        assert_eq!(
            parts,
            vec!["BDUSS=xx", "BAIDUID=yy", "PSTM=12345"],
            "names=[] should take every cookie in order"
        );
    }

    #[test]
    fn format_joined_cookies_filters_to_names_list() {
        let cookies = vec![
            ("BDUSS".to_string(), "xx".to_string()),
            ("BAIDUID".to_string(), "yy".to_string()),
            ("UNRELATED".to_string(), "drop_me".to_string()),
        ];
        let parts = format_joined_cookies(
            &["BDUSS".to_string(), "BAIDUID".to_string()],
            "{name}={value}",
            &cookies,
        );
        assert_eq!(parts, vec!["BDUSS=xx", "BAIDUID=yy"]);
    }

    #[test]
    fn format_joined_cookies_custom_fmt_template() {
        let cookies = vec![("BDUSS".to_string(), "abc".to_string())];
        let parts = format_joined_cookies(&[], "Cookie: {name}: {value}", &cookies);
        assert_eq!(parts, vec!["Cookie: BDUSS: abc"]);
    }

    #[test]
    fn format_joined_cookies_empty_input_returns_empty() {
        let parts = format_joined_cookies(&[], "{name}={value}", &[]);
        assert!(parts.is_empty());
    }

    #[test]
    fn cookie_domain_matches_exact_host() {
        assert!(cookie_domain_matches("baidu.com", "baidu.com"));
        assert!(cookie_domain_matches("BAIDU.COM", "baidu.com"));
    }

    #[test]
    fn cookie_domain_matches_strips_leading_dot() {
        // This is the case that wry 0.54's `cookies_for_url` botches:
        // NSHTTPCookie hands back `.baidu.com`, url.domain() returns
        // `baidu.com`, wry compares with `==` and drops every cookie.
        assert!(cookie_domain_matches(".baidu.com", "baidu.com"));
        assert!(cookie_domain_matches(".BAIDU.COM", "baidu.com"));
    }

    #[test]
    fn cookie_domain_matches_subdomain_under_target() {
        // Captures aiqicha-host-only cookies when the rule asks for
        // ".baidu.com" — needed because AQC sets some session cookies
        // host-only on aiqicha.baidu.com that ENScan_GO still needs.
        assert!(cookie_domain_matches("aiqicha.baidu.com", "baidu.com"));
        assert!(cookie_domain_matches(".aiqicha.baidu.com", "baidu.com"));
    }

    #[test]
    fn cookie_domain_matches_rejects_unrelated() {
        // Guardrail: a baidubcd.com cookie must NOT match baidu.com
        // (suffix match must be on a `.` boundary, not raw substring).
        assert!(!cookie_domain_matches("baidubcd.com", "baidu.com"));
        assert!(!cookie_domain_matches("evilbaidu.com", "baidu.com"));
        assert!(!cookie_domain_matches("notbaidu.com", "baidu.com"));
        assert!(!cookie_domain_matches("baidu.com.evil.com", "baidu.com"));
    }

    #[test]
    fn cookie_domain_matches_empty_inputs_return_false() {
        assert!(!cookie_domain_matches("", "baidu.com"));
        assert!(!cookie_domain_matches(".", "baidu.com"));
        assert!(!cookie_domain_matches("baidu.com", ""));
    }

    #[test]
    fn parse_js_value_title_decodes_ok_payload() {
        let payload = general_purpose::STANDARD.encode("secret-token");
        let title = format!("{JS_VALUE_TITLE_PREFIX}nonce-1:ok:{payload}");
        let (nonce, value) = parse_js_value_title(&title).expect("capture title should parse");
        assert_eq!(nonce, "nonce-1");
        assert_eq!(value.unwrap(), "secret-token");
    }

    #[test]
    fn parse_js_value_title_decodes_error_payload() {
        let payload = general_purpose::STANDARD.encode("missing selector");
        let title = format!("{JS_VALUE_TITLE_PREFIX}nonce-2:err:{payload}");
        let (nonce, value) = parse_js_value_title(&title).expect("capture title should parse");
        assert_eq!(nonce, "nonce-2");
        assert_eq!(value.unwrap_err(), "missing selector");
    }

    #[tokio::test]
    async fn gc_drops_only_old_terminal_sessions() {
        let eng = CaptureEngine::new();
        // Fresh terminal: should NOT be GC'd.
        let h_fresh = eng
            .register("t".into(), "fresh".into(), aqc_recipe())
            .await
            .unwrap();
        eng.cancel(&h_fresh.session_id).await.unwrap();

        // Stale terminal: backdate updated_at_ms to before the cutoff.
        let h_stale = eng
            .register("t".into(), "stale".into(), aqc_recipe())
            .await
            .unwrap();
        eng.cancel(&h_stale.session_id).await.unwrap();
        {
            let mut s = h_stale.inner.write().await;
            s.updated_at_ms = chrono::Utc::now().timestamp_millis() - GC_RETENTION_MS - 1;
        }

        // Non-terminal: should NOT be touched.
        let h_active = eng
            .register("t".into(), "active".into(), aqc_recipe())
            .await
            .unwrap();

        assert_eq!(eng.session_count().await, 3);
        eng.gc().await;
        assert_eq!(eng.session_count().await, 2, "only stale terminal removed");

        // active + fresh-cancelled should remain.
        assert!(eng.get(&h_fresh.session_id).await.is_ok());
        assert!(eng.get(&h_active.session_id).await.is_ok());
        assert!(eng.get(&h_stale.session_id).await.is_err());
    }
}
