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

use golish_integrations::error::{IntegrationError, IntegrationResult};
use golish_integrations::schema::{CaptureRecipe, CaptureRule};
use golish_integrations::types::{CaptureEventPayload, CaptureState, FailedRule};
use tokio::sync::RwLock;
use uuid::Uuid;

use super::data_dir;
use super::session::{CaptureSession, CaptureSessionHandle};

/// Garbage-collection threshold for terminal sessions: anything older
/// than 1 hour gets dropped from the registry. The frontend keeps its
/// own snapshot via the `_start` response so a missed status poll
/// after GC merely returns `CAPTURE_SESSION_NOT_FOUND` (handled by
/// the IPC layer as a 404).
const GC_RETENTION_MS: i64 = 3600 * 1000;

/// Tauri-managed singleton (`tauri::State<Arc<CaptureEngine>>`).
///
/// Sessions are keyed by their UUID-v4 `session_id`. The registry is
/// never persisted — a Golish restart wipes any in-flight sessions
/// (which is correct: their webviews are gone anyway).
pub struct CaptureEngine {
    sessions: RwLock<HashMap<String, CaptureSessionHandle>>,
}

impl Default for CaptureEngine {
    fn default() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
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

    /// Build the Tauri WebviewWindow for an already-registered session
    /// and wire its `on_navigation` callback to fire extraction when
    /// `success_url_pattern` matches.
    ///
    /// The webview's data store is isolated per-session
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
        let (sid, login_url, recipe) = {
            let s = handle.inner.read().await;
            (
                s.session_id.clone(),
                s.recipe.login_url.clone(),
                s.recipe.clone(),
            )
        };

        // 2. Reserve the per-session data directory (used on Linux /
        //    Windows for `data_directory`; on macOS we use
        //    `data_store_identifier` and the dir is empty / cleanup-only).
        let per_session_dir = data_dir::session_dir(&sid)?;

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

        let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
            .title(format!("Golish · 凭据抓取: {host_for_title}"))
            .inner_size(900.0, 700.0)
            .center()
            .focused(true)
            .visible(true);
        let builder = super::webview_isolation::apply_isolation(builder, &sid, &per_session_dir);
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
            (s.recipe.rules.clone(), s.tool_id.clone(), s.group_id.clone())
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
                    Some(
                        "[WEBVIEW_CREATE_FAILED] webview window vanished mid-extract"
                            .to_string(),
                    ),
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

        // Required failure short-circuit.
        if let Some((idx, reason)) = required_failure {
            self.transition_and_emit(
                app,
                session_id,
                CaptureState::Failed,
                Some(format!("[CAPTURE_RULE_FAILED] required rule #{idx}: {reason}")),
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
        self.transition_and_emit(app, session_id, next, None).await?;
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
        | CaptureRule::UrlQuery { required, .. } => *required,
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
///
/// P1 MVP supports only `Cookie`; the rest deliberately bail with a
/// "not yet implemented" reason so the UI surfaces the gap clearly
/// rather than silently succeeding.
async fn extract_one(
    win: &tauri::WebviewWindow,
    rule: &CaptureRule,
) -> Result<(String, String), String> {
    match rule {
        CaptureRule::Cookie {
            domain,
            name,
            target_field,
            ..
        } => {
            // Tauri's `cookies_for_url` is synchronous and on Windows
            // can deadlock when called from a sync command / event
            // handler. We always route through `spawn_blocking` to be
            // safe across platforms (negligible per-rule overhead).
            let host = domain.trim_start_matches('.').to_string();
            let url_str = format!("https://{host}/");
            let url = url_str
                .parse::<url::Url>()
                .map_err(|e| format!("invalid synthesized cookie URL {url_str}: {e}"))?;
            let win_clone = win.clone();
            let cookies = tokio::task::spawn_blocking(move || win_clone.cookies_for_url(url))
                .await
                .map_err(|e| format!("spawn_blocking join failed: {e}"))?
                .map_err(|e| format!("cookies_for_url failed: {e}"))?;
            let value = cookies
                .into_iter()
                .find(|c| c.name() == name.as_str())
                .ok_or_else(|| format!("cookie '{name}' not found in domain '{domain}'"))?
                .value()
                .to_string();
            Ok((target_field.clone(), value))
        }
        // P2 rules are explicitly out of scope for P1 MVP. The schema
        // parser accepts them so a forward-compatible JSON config
        // doesn't break, but the engine refuses to silently succeed.
        CaptureRule::CookieJoined { .. }
        | CaptureRule::LocalStorage { .. }
        | CaptureRule::SessionStorage { .. }
        | CaptureRule::PageContent { .. }
        | CaptureRule::UrlQuery { .. } => {
            Err("rule type not yet implemented in P1 MVP".to_string())
        }
    }
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
        assert!(s.error_message.is_none(), "error must not be stamped post-terminal");
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
