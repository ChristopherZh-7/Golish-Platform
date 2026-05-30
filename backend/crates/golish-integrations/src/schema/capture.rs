//! [`CaptureRecipe`] / [`CaptureRule`] — "click ⚡ to harvest creds from a
//! browser session".
//!
//! Architecture: docs/design/2026-05-21-credential-capture-engine.md

use serde::{Deserialize, Serialize};

/// A single capture recipe describing *how* Golish opens a webview,
/// detects login success, and extracts credentials into the schema's
/// declared fields.
///
/// Attached as [`super::IntegrationGroup::capture`]. When `None`, the
/// frontend does not render the ⚡ button and the user fills the form
/// manually (unchanged from pre-capture behavior).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureRecipe {
    /// HTTPS URL to navigate to in the capture webview. Must parse as
    /// `http://` or `https://` (validated server-side at schema load,
    /// see `resolver::validate_capture`).
    pub login_url: String,

    /// Regex applied to every navigation target URL. On match the
    /// engine triggers rule extraction. When `None`, extraction only
    /// runs when the user clicks the manual "complete" button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_url_pattern: Option<String>,

    /// Optional URL to navigate to *after* `success_url_pattern`
    /// matches but *before* running rules. Useful for sites that
    /// only show the API key on a settings page distinct from the
    /// login-success landing page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visit_url: Option<String>,

    /// Short Markdown / plain text shown in the confirm dialog. The
    /// frontend may fall back to the i18n key
    /// `integrations.capture.<tool>.<group>.hint` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,

    /// Hard timeout. Default 300 (5 min), max 900 (15 min). Values
    /// outside `[30, 900]` are clamped engine-side at session creation.
    #[serde(default = "default_capture_timeout")]
    pub timeout_secs: u32,

    /// Ordered list of extraction rules. Each rule writes into one
    /// `target_field` declared in the parent group's [`super::Field::key`].
    /// Order matters: a `PageContent` rule with `wait_ms` must come
    /// before any rule that depends on its DOM state.
    pub rules: Vec<CaptureRule>,
}

fn default_capture_timeout() -> u32 {
    300
}

/// One extraction action.
///
/// All variants reference a `target_field` that **must** match a
/// [`super::Field::key`] in the parent group (cross-validated at
/// schema-load time by `resolver::validate_capture`).
///
/// The capture engine supports cookie extraction plus JSON-driven
/// page/URL/storage extraction. Request-header capture is intentionally
/// not modelled here yet because Tauri/Wry does not expose a portable
/// external-HTTPS request interception API; that needs a separate
/// script-injection bridge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CaptureRule {
    /// Pull a single cookie from the webview's cookie store.
    Cookie {
        /// Cookie domain (with or without leading dot, e.g.
        /// `.aiqicha.baidu.com` or `aiqicha.baidu.com`).
        domain: String,
        /// Cookie name (exact match, case-sensitive).
        name: String,
        /// [`super::Field::key`] to write into.
        target_field: String,
        /// When `true` and the cookie is missing, the whole capture
        /// is marked `Failed`; otherwise it's marked `Partial`.
        #[serde(default = "default_true_capture")]
        required: bool,
    },

    /// Pull multiple cookies, format each via `fmt`, then join with
    /// `sep`. Used by sites that expect a manually-joined
    /// `name1=v1; name2=v2` cookie header (e.g. ENScan TYC).
    ///
    /// `required_names` is the **login-state proof set**: if any name in
    /// it is missing from the live cookie jar the rule fails (the
    /// capture engine surfaces it as a soft failure and waits for the
    /// next navigation rather than persisting an incomplete header).
    /// Used when `success_url_pattern` is necessarily loose (e.g. baidu
    /// drops the user back on `aiqicha.baidu.com/` root after two-factor
    /// auth) and we need an independent signal that the user actually
    /// finished logging in. Defaults to empty = no enforcement.
    ///
    /// `min_count` is a looser login-state proof for providers whose
    /// stable login cookie names are not known yet. It counts the
    /// cookies selected by `names` (or all domain cookies when
    /// `names=[]`) and lets capture soft-retry until enough cookies are
    /// present. Defaults to 0 = no minimum.
    CookieJoined {
        domain: String,
        names: Vec<String>,
        #[serde(default = "default_cookie_sep")]
        sep: String,
        #[serde(default = "default_cookie_fmt")]
        fmt: String,
        target_field: String,
        #[serde(default = "default_true_capture")]
        required: bool,
        #[serde(default)]
        required_names: Vec<String>,
        #[serde(default)]
        min_count: usize,
    },

    /// Read `localStorage[key]` via `WebviewWindow::eval_with_callback`.
    LocalStorage {
        key: String,
        target_field: String,
        #[serde(default = "default_true_capture")]
        required: bool,
    },

    /// Read `sessionStorage[key]` via `WebviewWindow::eval_with_callback`.
    SessionStorage {
        key: String,
        target_field: String,
        #[serde(default = "default_true_capture")]
        required: bool,
    },

    /// Read `document.querySelector(selector).textContent` (or the
    /// `attribute` value when set).
    PageContent {
        selector: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attribute: Option<String>,
        #[serde(default = "default_wait_ms")]
        wait_ms: u32,
        target_field: String,
        #[serde(default = "default_true_capture")]
        required: bool,
    },

    /// Read the named query parameter from the current page URL.
    UrlQuery {
        name: String,
        target_field: String,
        #[serde(default = "default_true_capture")]
        required: bool,
    },

    /// Read a JavaScript-set request header observed by the capture
    /// webview's injected fetch/XMLHttpRequest monitor. `url_pattern`
    /// is an optional regex applied to the request URL; when absent the
    /// most recent matching header by name is used.
    RequestHeader {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url_pattern: Option<String>,
        target_field: String,
        #[serde(default = "default_true_capture")]
        required: bool,
    },
}

fn default_true_capture() -> bool {
    true
}

fn default_cookie_sep() -> String {
    "; ".to_string()
}

fn default_cookie_fmt() -> String {
    "{name}={value}".to_string()
}

fn default_wait_ms() -> u32 {
    3000
}

impl CaptureRule {
    /// Returns the `target_field` this rule writes into.
    pub fn target_field(&self) -> &str {
        match self {
            Self::Cookie { target_field, .. }
            | Self::CookieJoined { target_field, .. }
            | Self::LocalStorage { target_field, .. }
            | Self::SessionStorage { target_field, .. }
            | Self::PageContent { target_field, .. }
            | Self::UrlQuery { target_field, .. }
            | Self::RequestHeader { target_field, .. } => target_field,
        }
    }

    /// Whether this rule's failure is fatal to the capture session.
    pub fn required(&self) -> bool {
        match self {
            Self::Cookie { required, .. }
            | Self::CookieJoined { required, .. }
            | Self::LocalStorage { required, .. }
            | Self::SessionStorage { required, .. }
            | Self::PageContent { required, .. }
            | Self::UrlQuery { required, .. }
            | Self::RequestHeader { required, .. } => *required,
        }
    }

    /// Short identifier for logging / error reporting (e.g.
    /// `"cookie"` / `"page_content"`).
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Cookie { .. } => "cookie",
            Self::CookieJoined { .. } => "cookie_joined",
            Self::LocalStorage { .. } => "local_storage",
            Self::SessionStorage { .. } => "session_storage",
            Self::PageContent { .. } => "page_content",
            Self::UrlQuery { .. } => "url_query",
            Self::RequestHeader { .. } => "request_header",
        }
    }
}
