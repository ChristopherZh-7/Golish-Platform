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
//! Submodules: [`storage`] (persistence variants), [`test_kind`]
//! (connectivity-test recipes), [`capture`] (browser credential-capture
//! recipes). All public types are re-exported here so `schema::Storage`
//! etc. stay stable.
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

mod capture;
mod storage;
mod test_kind;

pub use capture::*;
pub use storage::*;
pub use test_kind::*;

#[cfg(test)]
mod tests;

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
