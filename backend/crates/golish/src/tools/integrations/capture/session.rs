//! In-memory representation of one capture session.
//!
//! A [`CaptureSession`] is the mutable state behind a `Arc<RwLock<_>>`
//! ([`CaptureSessionHandle`]) so the engine, the navigation callback
//! (Tauri main loop), and the TTL watcher can all touch it without
//! contending on a single global lock.

use std::sync::Arc;
use std::time::Instant;

use golish_integrations::schema::CaptureRecipe;
use golish_integrations::types::{CaptureSessionInfo, CaptureState, FailedRule};
use tokio::sync::RwLock;

/// Effective minimum / maximum capture timeout (in seconds). Values
/// supplied in the recipe are clamped to this range at session
/// creation time so a typo can't lock the user out for an hour or
/// leave the webview open too briefly to log in.
pub(crate) const TIMEOUT_MIN_SECS: u32 = 30;
pub(crate) const TIMEOUT_MAX_SECS: u32 = 900;

/// One in-flight capture session's mutable state.
#[derive(Debug, Clone)]
pub struct CaptureSession {
    pub session_id: String,
    pub tool_id: String,
    pub group_id: String,
    pub recipe: CaptureRecipe,
    pub state: CaptureState,
    pub captured_fields: Vec<String>,
    pub failed_rules: Vec<FailedRule>,
    pub error_message: Option<String>,
    /// Monotonic-clock anchor for TTL math. Not serialized to UI.
    pub started_at: Instant,
    /// `started_at` as Unix milliseconds — what the frontend countdown
    /// compares against `Date.now()`.
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
    /// Effective TTL after clamping to `[TIMEOUT_MIN_SECS, TIMEOUT_MAX_SECS]`.
    pub timeout_secs: u32,
}

impl CaptureSession {
    pub fn new(
        session_id: String,
        tool_id: String,
        group_id: String,
        recipe: CaptureRecipe,
    ) -> Self {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let timeout_secs = recipe.timeout_secs.clamp(TIMEOUT_MIN_SECS, TIMEOUT_MAX_SECS);
        Self {
            session_id,
            tool_id,
            group_id,
            recipe,
            state: CaptureState::WaitingLogin,
            captured_fields: Vec::new(),
            failed_rules: Vec::new(),
            error_message: None,
            started_at: Instant::now(),
            started_at_ms: now_ms,
            updated_at_ms: now_ms,
            timeout_secs,
        }
    }

    /// Updates the state and the `updated_at_ms` timestamp.
    pub fn transition(&mut self, next: CaptureState) {
        self.state = next;
        self.updated_at_ms = chrono::Utc::now().timestamp_millis();
    }

    /// Builds the serializable snapshot the IPC layer hands back to
    /// the frontend. `expires_at` is `None` once we're terminal so the
    /// UI knows to stop the countdown.
    pub fn info(&self) -> CaptureSessionInfo {
        let expires_at = if self.state.is_terminal() {
            None
        } else {
            Some(self.started_at_ms + (self.timeout_secs as i64) * 1000)
        };
        CaptureSessionInfo {
            session_id: self.session_id.clone(),
            tool_id: self.tool_id.clone(),
            group_id: self.group_id.clone(),
            state: self.state,
            login_url: self.recipe.login_url.clone(),
            expected_fields: self
                .recipe
                .rules
                .iter()
                .map(|r| r.target_field().to_string())
                .collect(),
            captured_fields: self.captured_fields.clone(),
            failed_rules: self.failed_rules.clone(),
            error_message: self.error_message.clone(),
            expires_at,
            updated_at: self.updated_at_ms,
        }
    }
}

/// Shared handle: an `Arc<RwLock<CaptureSession>>` plus the immutable
/// session_id (cached so we don't lock to read it).
#[derive(Debug, Clone)]
pub struct CaptureSessionHandle {
    pub session_id: String,
    pub inner: Arc<RwLock<CaptureSession>>,
}

impl CaptureSessionHandle {
    pub fn new(session: CaptureSession) -> Self {
        Self {
            session_id: session.session_id.clone(),
            inner: Arc::new(RwLock::new(session)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_integrations::schema::{CaptureRecipe, CaptureRule};

    fn aqc_recipe(timeout_secs: u32) -> CaptureRecipe {
        CaptureRecipe {
            login_url: "https://aiqicha.baidu.com".into(),
            success_url_pattern: None,
            visit_url: None,
            instructions: None,
            timeout_secs,
            rules: vec![CaptureRule::Cookie {
                domain: ".aiqicha.baidu.com".into(),
                name: "BDUSS".into(),
                target_field: "cookies.aqc".into(),
                required: true,
            }],
        }
    }

    #[test]
    fn new_session_starts_waiting_login_with_clamped_timeout() {
        let s = CaptureSession::new(
            "sid".into(),
            "enscan-go".into(),
            "aqc".into(),
            aqc_recipe(10),
        );
        assert_eq!(s.state, CaptureState::WaitingLogin);
        assert_eq!(s.timeout_secs, TIMEOUT_MIN_SECS, "clamped up to MIN");
        assert!(s.captured_fields.is_empty());

        let s = CaptureSession::new(
            "sid".into(),
            "enscan-go".into(),
            "aqc".into(),
            aqc_recipe(10_000),
        );
        assert_eq!(s.timeout_secs, TIMEOUT_MAX_SECS, "clamped down to MAX");
    }

    #[test]
    fn transition_updates_state_and_timestamp() {
        let mut s = CaptureSession::new(
            "sid".into(),
            "t".into(),
            "g".into(),
            aqc_recipe(60),
        );
        let t0 = s.updated_at_ms;
        std::thread::sleep(std::time::Duration::from_millis(5));
        s.transition(CaptureState::Extracting);
        assert_eq!(s.state, CaptureState::Extracting);
        assert!(s.updated_at_ms >= t0);
    }

    #[test]
    fn info_omits_expires_at_when_terminal() {
        let mut s = CaptureSession::new(
            "sid".into(),
            "t".into(),
            "g".into(),
            aqc_recipe(60),
        );
        let info = s.info();
        assert!(info.expires_at.is_some(), "non-terminal has expires_at");
        s.transition(CaptureState::Captured);
        let info = s.info();
        assert!(info.expires_at.is_none(), "terminal omits expires_at");
    }

    #[test]
    fn info_expected_fields_uses_target_field_helper() {
        let s = CaptureSession::new(
            "sid".into(),
            "t".into(),
            "g".into(),
            aqc_recipe(60),
        );
        let info = s.info();
        assert_eq!(info.expected_fields, vec!["cookies.aqc".to_string()]);
    }
}
