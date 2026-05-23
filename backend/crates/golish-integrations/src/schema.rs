//! Schema types describing one external-service integration.
//!
//! An [`IntegrationSchema`] is the **self-description** of one external
//! service that Golish needs to talk to. It lives next to the
//! integration's `toolsconfig` JSON (for tool-managed integrations like
//! ENScan_GO) or is constructed in code (for `IntelProvider`
//! implementations).
//!
//! The schema fully describes:
//! 1. Where the secrets are stored ([`Storage`]).
//! 2. What groups of fields the user fills ([`IntegrationGroup`]).
//! 3. How to validate the configured credentials ([`TestKind`]).
//!
//! It carries **no** secret values itself — values are loaded /
//! persisted by a [`crate::traits::StorageBackend`].
//!
//! ## Wire-format example (tool JSON)
//!
//! ```jsonc
//! {
//!   "tool": {
//!     "id": "enscan-go",
//!     "integration": {
//!       "category": "enterprise-intel",
//!       "display_name": "ENScan_GO 企业情报",
//!       "storage": {
//!         "type": "external_file",
//!         "external_file": {
//!           "path": "~/.config/enscan/config.yaml",
//!           "format": "yaml",
//!           "preserve_unknown_keys": true
//!         }
//!       },
//!       "groups": [
//!         {
//!           "id": "aqc",
//!           "name": "爱企查 (AQC)",
//!           "fields": [
//!             { "key": "cookies.aqc", "label": "Cookie",
//!               "type": "secret_textarea", "required": true }
//!           ]
//!         }
//!       ]
//!     }
//!   }
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A complete integration descriptor: where to store secrets, what
/// fields make up each group, and how to test connectivity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntegrationSchema {
    /// UI category id (e.g. `"asm-cyberspace"`, `"enterprise-intel"`,
    /// `"code-host"`). The frontend uses this to render the side-nav.
    pub category: String,

    /// Human-readable name shown on the card header.
    pub display_name: String,

    /// Short one-line description shown under the card title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Where credentials are persisted.
    pub storage: Storage,

    /// One integration can declare multiple credential groups
    /// (e.g. ENScan_GO has aqc / tyc / kc / rb / miit). Each group
    /// is rendered as its own collapsible sub-section.
    pub groups: Vec<IntegrationGroup>,

    /// Optional documentation URL shown on the card header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_url: Option<String>,
}

/// A logical group of fields the user fills together.
///
/// Examples:
/// - 0.zone: one group `"default"` with field `api_key`.
/// - ENScan/TYC: one group `"tyc"` with three fields: cookie, tycid,
///   auth_token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntegrationGroup {
    /// Stable id, used in `tags=["integration-group", tool_id, group_id]`
    /// and as a path component for [`Storage::ExternalFile`] writes.
    pub id: String,

    /// Display name (e.g. "爱企查 (AQC)").
    pub name: String,

    /// Optional longer description / setup instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Optional emoji or icon hint (one short string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Optional help link for THIS group specifically (e.g. how to
    /// extract AQC cookie). Group-level `help_url` overrides the
    /// schema-level one when both are present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_url: Option<String>,

    /// Ordered list of fields the user fills.
    pub fields: Vec<Field>,

    /// Optional connectivity-test recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<TestKind>,

    /// Optional auto-capture recipe ("click ⚡ to harvest from
    /// browser"). When `None`, the frontend hides the ⚡ button and
    /// the user must fill the form manually.
    ///
    /// See `docs/design/2026-05-21-credential-capture-engine.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureRecipe>,
}

/// Single form field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Field {
    /// Dotted key path. For [`Storage::ExternalFile`] it determines
    /// where the value lands in the YAML/JSON tree
    /// (`"cookies.aqc"` → `cookies: { aqc: <value> }`).
    /// For [`Storage::Vault`] it's appended to the vault entry's
    /// `name` (`"<tool>.<group>.<key>"`) so multiple fields in the
    /// same group don't collide.
    pub key: String,

    /// Display label (e.g. "Cookie", "API Key").
    pub label: String,

    /// Input renderer hint.
    #[serde(rename = "type")]
    pub field_type: FieldType,

    /// Placeholder text shown when empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,

    /// When true, frontend marks the field with a required indicator
    /// and the backend rejects writes that leave it blank.
    #[serde(default)]
    pub required: bool,

    /// Optional rows hint for `secret_textarea` / `textarea`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u32>,

    /// Optional dropdown options when `field_type == "select"`.
    /// Each option is `{ "value": "...", "label": "..." }`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,

    /// Optional regex the value must match (server-side only — the
    /// frontend can use it as a UX hint but server is authoritative).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

/// Field input renderer hint. The frontend has a matching `<SecretInput>`
/// / `<UrlInput>` / etc. component per variant.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// Single-line password input with reveal toggle.
    SecretText,
    /// Multi-line textarea (cookies, certificates) with reveal toggle.
    SecretTextarea,
    /// Single-line non-secret text input.
    Text,
    /// URL input with light validation.
    Url,
    /// Numeric port input (1-65535).
    Port,
    /// Dropdown with `options[]`.
    Select,
    /// Checkbox / toggle.
    Boolean,
    /// Composite proxy field (URL or host+port+auth depending on
    /// implementation). Always non-secret on the URL surface, secrets
    /// go to dependent fields.
    Proxy,
}

impl FieldType {
    /// Whether values of this type need encryption / reveal-toggle UI.
    pub fn is_secret(self) -> bool {
        matches!(self, Self::SecretText | Self::SecretTextarea)
    }
}

/// One option in a dropdown.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

/// Where credentials are persisted.
///
/// Tag-discriminated union with a **nested** payload per variant
/// (easier to read in JSON / YAML hand-edited by humans):
///
/// ```jsonc
/// { "type": "vault", "vault": { "extra_tags": [...] } }
/// { "type": "external_file",
///   "external_file": { "path": "...", "format": "yaml", ... } }
/// { "type": "settings", "settings": { "key": "network.github_token" } }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Storage {
    /// Stored encrypted in the `vault_entries` table. Each field in a
    /// group becomes one vault row, aggregated by
    /// `tags=["integration-group", <tool>, <group>]`.
    Vault {
        #[serde(default)]
        vault: VaultStorage,
    },

    /// Rendered into a file the external process reads (e.g.
    /// `~/.config/enscan/config.yaml`). The integration crate never
    /// keeps a separate copy — the file is authoritative.
    ExternalFile { external_file: ExternalFileStorage },

    /// Written through [`crate::traits::StorageBackend`] into the
    /// existing `golish settings.toml` at the given dotted path
    /// (e.g. `network.github_token`).
    Settings { settings: SettingsStorage },
}

impl Storage {
    /// Convenience constructor for `Storage::Vault` with default tags.
    pub fn vault_default() -> Self {
        Self::Vault {
            vault: VaultStorage::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct VaultStorage {
    /// Extra tags to attach to vault rows on top of the default
    /// `["integration-group", <tool>, <group>]`. Optional, useful for
    /// migration markers (`"data-source"` / `"intel-provider"`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalFileStorage {
    /// File path with `~` expanded at write time. Required.
    pub path: String,

    /// File format. Determines parser / serializer used.
    #[serde(default = "default_yaml_format")]
    pub format: ExternalFileFormat,

    /// When true, parse existing file and merge new values into it
    /// rather than overwriting (so user-added keys outside our schema
    /// survive a Golish write).
    #[serde(default = "default_true")]
    pub preserve_unknown_keys: bool,

    /// When true, copy the existing file to
    /// `<path>.bak.<YYYYMMDD-HHMMSS>` before writing the new one.
    /// At most 3 backups are kept (oldest rotated out).
    #[serde(default = "default_true")]
    pub backup_on_write: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ExternalFileFormat {
    Yaml,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingsStorage {
    /// Dotted setting path (e.g. `"network.github_token"`). Resolved
    /// via the existing `SettingsManager`.
    pub key: String,
}

fn default_yaml_format() -> ExternalFileFormat {
    ExternalFileFormat::Yaml
}

fn default_true() -> bool {
    true
}

/// How to verify the configured credentials actually work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TestKind {
    /// Delegate to a provider-defined test. Used by `IntelProvider`
    /// implementors that already have their own `test_connection`.
    Builtin,

    /// Spawn a command. `{{exec}}` is substituted with the tool's
    /// resolved executable path. Stdout is matched against
    /// `ok_regex` / `fail_regex`.
    Exec {
        /// Shell-style command template.
        cmd: String,
        /// On match → [`crate::types::HealthStatus::Healthy`].
        ok_regex: String,
        /// On match (before checking ok_regex) →
        /// [`crate::types::HealthStatus::Invalid`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fail_regex: Option<String>,
        #[serde(default = "default_timeout_30")]
        timeout_secs: u32,
    },

    /// Issue an HTTP request and check the status code.
    Http {
        method: String,
        /// URL template, supports `{{value:field_key}}` substitution.
        url: String,
        /// Header templates, supports the same substitution.
        #[serde(default)]
        headers: HashMap<String, String>,
        /// Inclusive `[lo, hi]`. Default 200..=299.
        #[serde(default = "default_ok_range")]
        ok_status_range: (u16, u16),
        #[serde(default = "default_timeout_30")]
        timeout_secs: u32,
    },
}

fn default_timeout_30() -> u32 {
    30
}

fn default_ok_range() -> (u16, u16) {
    (200, 299)
}

// ────────────────────────────────────────────────────────────────────────
// CaptureRecipe — "click ⚡ to harvest creds from a browser session"
//
// Architecture: docs/design/2026-05-21-credential-capture-engine.md
// ────────────────────────────────────────────────────────────────────────

/// A single capture recipe describing *how* Golish opens a webview,
/// detects login success, and extracts credentials into the schema's
/// declared fields.
///
/// Attached as [`IntegrationGroup::capture`]. When `None`, the
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
    /// `target_field` declared in the parent group's [`Field::key`].
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
/// [`Field::key`] in the parent group (cross-validated at
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
        /// [`Field::key`] to write into.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_serde_round_trip_minimal() {
        let s = IntegrationSchema {
            category: "asm-cyberspace".into(),
            display_name: "0.zone".into(),
            description: None,
            storage: Storage::vault_default(),
            groups: vec![IntegrationGroup {
                id: "default".into(),
                name: "API Key".into(),
                description: None,
                icon: None,
                help_url: None,
                fields: vec![Field {
                    key: "api_key".into(),
                    label: "API Key".into(),
                    field_type: FieldType::SecretText,
                    placeholder: None,
                    required: true,
                    rows: None,
                    options: vec![],
                    pattern: None,
                }],
                test: Some(TestKind::Builtin),
                capture: None,
            }],
            help_url: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: IntegrationSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn storage_external_file_yaml() {
        let raw = r#"{
            "type": "external_file",
            "external_file": {
                "path": "~/.config/enscan/config.yaml",
                "format": "yaml",
                "preserve_unknown_keys": true,
                "backup_on_write": true
            }
        }"#;
        let s: Storage = serde_json::from_str(raw).unwrap();
        match s {
            Storage::ExternalFile { external_file: ef } => {
                assert_eq!(ef.path, "~/.config/enscan/config.yaml");
                assert_eq!(ef.format, ExternalFileFormat::Yaml);
                assert!(ef.preserve_unknown_keys);
                assert!(ef.backup_on_write);
            }
            _ => panic!("expected ExternalFile, got {:?}", s),
        }
    }

    #[test]
    fn storage_settings() {
        let raw = r#"{"type":"settings","settings":{"key":"network.github_token"}}"#;
        let s: Storage = serde_json::from_str(raw).unwrap();
        match s {
            Storage::Settings { settings } => {
                assert_eq!(settings.key, "network.github_token");
            }
            _ => panic!("expected Settings"),
        }
    }

    #[test]
    fn storage_vault_default_round_trip() {
        let s = Storage::vault_default();
        let json = serde_json::to_string(&s).unwrap();
        // Vault with empty extra_tags should still serialize cleanly.
        let back: Storage = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn test_kind_exec() {
        let raw = r#"{
            "kind": "exec",
            "cmd": "{{exec}} -n 小米 -type aqc",
            "ok_regex": "company_name",
            "timeout_secs": 10
        }"#;
        let t: TestKind = serde_json::from_str(raw).unwrap();
        match t {
            TestKind::Exec {
                cmd,
                ok_regex,
                fail_regex,
                timeout_secs,
            } => {
                assert!(cmd.contains("{{exec}}"));
                assert_eq!(ok_regex, "company_name");
                assert_eq!(fail_regex, None);
                assert_eq!(timeout_secs, 10);
            }
            _ => panic!("expected Exec"),
        }
    }

    #[test]
    fn test_kind_http() {
        let raw = r#"{
            "kind": "http",
            "method": "GET",
            "url": "https://api.github.com/user",
            "headers": { "Authorization": "Bearer {{value:token}}" }
        }"#;
        let t: TestKind = serde_json::from_str(raw).unwrap();
        match t {
            TestKind::Http {
                method,
                url,
                headers,
                ok_status_range,
                timeout_secs,
            } => {
                assert_eq!(method, "GET");
                assert_eq!(url, "https://api.github.com/user");
                assert!(headers.contains_key("Authorization"));
                assert_eq!(ok_status_range, (200, 299));
                assert_eq!(timeout_secs, 30);
            }
            _ => panic!("expected Http"),
        }
    }

    #[test]
    fn field_type_is_secret() {
        assert!(FieldType::SecretText.is_secret());
        assert!(FieldType::SecretTextarea.is_secret());
        assert!(!FieldType::Text.is_secret());
        assert!(!FieldType::Url.is_secret());
        assert!(!FieldType::Boolean.is_secret());
    }

    // ────────────────────────────────────────────────────────────────
    // CaptureRecipe / CaptureRule tests (Phase 1 T1.1)
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn capture_recipe_round_trip_cookie_rule() {
        let raw = r#"{
            "login_url": "https://aiqicha.baidu.com",
            "success_url_pattern": "aiqicha\\.baidu\\.com/(home|company)",
            "timeout_secs": 300,
            "rules": [
                { "type": "cookie", "domain": ".aiqicha.baidu.com",
                  "name": "BDUSS", "target_field": "cookies.aqc" }
            ]
        }"#;
        let r: CaptureRecipe = serde_json::from_str(raw).unwrap();
        assert_eq!(r.login_url, "https://aiqicha.baidu.com");
        assert_eq!(r.timeout_secs, 300);
        assert_eq!(r.rules.len(), 1);
        match &r.rules[0] {
            CaptureRule::Cookie {
                domain,
                name,
                target_field,
                required,
            } => {
                assert_eq!(domain, ".aiqicha.baidu.com");
                assert_eq!(name, "BDUSS");
                assert_eq!(target_field, "cookies.aqc");
                assert!(*required);
            }
            other => panic!("expected Cookie, got {other:?}"),
        }
        let back: CaptureRecipe =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn capture_recipe_defaults() {
        // Need r##"..."## because the JSON contains `#api-key` which
        // would otherwise close the r#"..."# delimiter at the `"#`.
        let raw = r##"{
            "login_url": "https://fofa.info",
            "rules": [
                { "type": "page_content", "selector": "#api-key",
                  "target_field": "api_key" }
            ]
        }"##;
        let r: CaptureRecipe = serde_json::from_str(raw).unwrap();
        assert_eq!(r.timeout_secs, 300, "default timeout_secs is 300");
        match &r.rules[0] {
            CaptureRule::PageContent {
                wait_ms, required, ..
            } => {
                assert_eq!(*wait_ms, 3000, "default wait_ms is 3000");
                assert!(*required, "default required is true");
            }
            other => panic!("expected PageContent, got {other:?}"),
        }
    }

    #[test]
    fn capture_rule_helper_methods() {
        let cookie = CaptureRule::Cookie {
            domain: ".x.com".into(),
            name: "Y".into(),
            target_field: "f.a".into(),
            required: true,
        };
        assert_eq!(cookie.target_field(), "f.a");
        assert!(cookie.required());
        assert_eq!(cookie.kind_name(), "cookie");

        let cookie_joined = CaptureRule::CookieJoined {
            domain: ".x.com".into(),
            names: vec!["a".into(), "b".into()],
            sep: "; ".into(),
            fmt: "{name}={value}".into(),
            target_field: "f.b".into(),
            required: false,
            required_names: vec![],
            min_count: 0,
        };
        assert_eq!(cookie_joined.target_field(), "f.b");
        assert!(!cookie_joined.required());
        assert_eq!(cookie_joined.kind_name(), "cookie_joined");

        let ls = CaptureRule::LocalStorage {
            key: "k".into(),
            target_field: "f.c".into(),
            required: true,
        };
        assert_eq!(ls.kind_name(), "local_storage");

        let ss = CaptureRule::SessionStorage {
            key: "k".into(),
            target_field: "f.d".into(),
            required: true,
        };
        assert_eq!(ss.kind_name(), "session_storage");

        let pc = CaptureRule::PageContent {
            selector: "#x".into(),
            attribute: Some("data-token".into()),
            wait_ms: 1000,
            target_field: "f.e".into(),
            required: true,
        };
        assert_eq!(pc.kind_name(), "page_content");

        let uq = CaptureRule::UrlQuery {
            name: "code".into(),
            target_field: "f.f".into(),
            required: true,
        };
        assert_eq!(uq.kind_name(), "url_query");
    }

    #[test]
    fn integration_group_capture_defaults_to_none() {
        let raw = r#"{
            "id": "default",
            "name": "API Key",
            "fields": [
                { "key": "api_key", "label": "API Key", "type": "secret_text" }
            ]
        }"#;
        let g: IntegrationGroup = serde_json::from_str(raw).unwrap();
        assert!(g.capture.is_none(), "capture defaults to None when absent");
        assert!(g.test.is_none(), "(sanity) test also still optional");
    }

    #[test]
    fn integration_group_with_capture_round_trip() {
        let g = IntegrationGroup {
            id: "aqc".into(),
            name: "爱企查".into(),
            description: None,
            icon: None,
            help_url: None,
            fields: vec![Field {
                key: "cookies.aqc".into(),
                label: "Cookie".into(),
                field_type: FieldType::SecretTextarea,
                placeholder: None,
                required: true,
                rows: None,
                options: vec![],
                pattern: None,
            }],
            test: None,
            capture: Some(CaptureRecipe {
                login_url: "https://aiqicha.baidu.com".into(),
                success_url_pattern: Some(r"aiqicha\.baidu\.com".into()),
                visit_url: None,
                instructions: None,
                timeout_secs: 300,
                rules: vec![CaptureRule::Cookie {
                    domain: ".aiqicha.baidu.com".into(),
                    name: "BDUSS".into(),
                    target_field: "cookies.aqc".into(),
                    required: true,
                }],
            }),
        };
        let json = serde_json::to_string(&g).unwrap();
        let back: IntegrationGroup = serde_json::from_str(&json).unwrap();
        assert_eq!(g, back);
    }
}
