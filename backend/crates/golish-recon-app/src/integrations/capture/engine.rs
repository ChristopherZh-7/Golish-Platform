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
//!
//! ## Module layout
//!   - this file: consts, the [`CaptureEngine`] struct, and its session
//!     lifecycle / state-machine / webview methods.
//!   - [`extract`]: the `try_extract` rule runner + navigation handler
//!     and per-rule extraction (`extract_one`).
//!   - [`helpers`]: low-level webview JS-eval, cookie access, and the
//!     storage-backend persistence bridge.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use golish_integrations::error::{IntegrationError, IntegrationResult};
use golish_integrations::schema::CaptureRecipe;
use golish_integrations::types::{CaptureEventPayload, CaptureState};
use tokio::sync::{oneshot, RwLock};
use uuid::Uuid;

use super::data_dir;
use super::session::{CaptureSession, CaptureSessionHandle};

mod extract;
mod helpers;

pub(crate) use extract::*;
pub(crate) use helpers::*;

/// Garbage-collection threshold for terminal sessions: anything older
/// than 1 hour gets dropped from the registry. The frontend keeps its
/// own snapshot via the `_start` response so a missed status poll
/// after GC merely returns `CAPTURE_SESSION_NOT_FOUND` (handled by
/// the IPC layer as a 404).
pub(crate) const GC_RETENTION_MS: i64 = 3600 * 1000;
pub(crate) const JS_VALUE_TITLE_PREFIX: &str = "__GOLISH_CAPTURE_VALUE__:";
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

#[cfg(test)]
mod tests;
