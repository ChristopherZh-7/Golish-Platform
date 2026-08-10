//! Shared data types for the intel-providers crate.

use chrono::{DateTime, Utc};
use golish_integrations::schema::{
    Field, FieldType, IntegrationGroup, IntegrationSchema, Storage, TestKind, VaultStorage,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Build a single-field `secret_text` "API Key" Vault integration
/// schema — the shape every cyberspace-mapping ASM provider in this
/// crate uses for its credentials. The connectivity test path is
/// declared as [`TestKind::Builtin`] so the IPC facade dispatches
/// the test to `IntelProvider::test_connection` (Phase 5+ hook).
///
/// Callers pass:
/// - `display_name`: card title in the Settings UI (translated copy
///   lives next to the same string in i18n files).
/// - `description`: one-line subtitle.
/// - `placeholder`: input placeholder hint (e.g. `"email|key"` for
///   FOFA's two-piece key format).
pub fn api_key_integration_schema(
    display_name: &str,
    description: &str,
    placeholder: Option<&str>,
    signup_url: Option<&str>,
) -> IntegrationSchema {
    IntegrationSchema {
        category: "asm-cyberspace".into(),
        display_name: display_name.into(),
        description: Some(description.into()),
        // Surface the provider's signup page on the schema so the UI
        // can render a single "Sign up / API key" link on the card
        // header — most users land here because they need to obtain
        // the key in the first place.
        help_url: signup_url.map(|s| s.to_string()),
        storage: Storage::Vault {
            vault: VaultStorage {
                // Tag the row so the legacy IntelProvidersSettings UI
                // still recognises it during the migration period
                // (see docs/design/2026-05-21-integrations.md §6.1).
                extra_tags: vec!["intel-provider".into()],
            },
        },
        groups: vec![IntegrationGroup {
            id: "default".into(),
            name: "API Key".into(),
            description: None,
            icon: None,
            help_url: signup_url.map(|s| s.to_string()),
            test: Some(TestKind::Builtin),
            capture: None,
            fields: vec![Field {
                key: "api_key".into(),
                label: "API Key".into(),
                field_type: FieldType::SecretText,
                placeholder: placeholder.map(|s| s.to_string()),
                required: true,
                rows: None,
                options: vec![],
                pattern: None,
            }],
        }],
    }
}

/// Query category. Each provider declares which subset of these it supports.
///
/// The names roughly map to the `query_type` parameter of various ASM
/// platforms (0.zone uses `site` / `domain` / `email` / `apk` / `app` /
/// `code` / `member` / `org` / `branch`; FOFA / Quake use similar but
/// distinct vocabularies).
/// The enum is intentionally a superset; provider impls return
/// `IntelError::UnsupportedQueryType` for variants they don't handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryType {
    /// Information system / surface (ip + url + title + status + tech).
    Site,
    /// Subdomain enumeration.
    Domain,
    /// Email / contact discovery.
    Email,
    /// Mobile app (APK / iOS app metadata).
    Apk,
    /// Sensitive directories / paths.
    Sensitive,
    /// Code / document leakage.
    Code,
    /// Employee / member identity.
    Member,
    /// Organization / company profile.
    Org,
    /// Branch / subsidiary profile.
    Branch,
    /// Darknet intelligence.
    Darknet,
    /// Certificate transparency.
    Cert,
    /// ASN / network range.
    Asn,
    /// IP-range / CIDR.
    Cidr,
}

impl QueryType {
    /// Convert to the wire-format string most providers use.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Site => "site",
            Self::Domain => "domain",
            Self::Email => "email",
            Self::Apk => "apk",
            Self::Sensitive => "sensitive",
            Self::Code => "code",
            Self::Member => "member",
            Self::Org => "org",
            Self::Branch => "branch",
            Self::Darknet => "darknet",
            Self::Cert => "cert",
            Self::Asn => "asn",
            Self::Cidr => "cidr",
        }
    }
}

/// Escape one semantic value for placement inside a provider-owned quoted
/// literal. This function never accepts or emits a complete provider query;
/// provider modules prepend their fixed field/operator after escaping.
pub fn escape_provider_literal(value: &str) -> crate::IntelResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(crate::IntelError::Other(
            "semantic provider literal is empty".to_string(),
        ));
    }
    if value.chars().count() > 512 {
        return Err(crate::IntelError::Other(
            "semantic provider literal exceeds 512 characters".to_string(),
        ));
    }

    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            control if control.is_control() => {
                use std::fmt::Write as _;
                write!(&mut escaped, "\\u{:04x}", control as u32)
                    .expect("writing to String cannot fail");
            }
            other => escaped.push(other),
        }
    }
    Ok(escaped)
}

/// Static metadata about an intel provider. Used by Settings UI to render
/// a card per provider, link to signup / docs, and show free-tier info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMeta {
    /// Stable identifier, matches `IntelProvider::id()`. Also used as
    /// the vault entry name prefix.
    pub id: String,
    /// Human-readable display name (e.g. "0.zone（零零信安）").
    pub display_name: String,
    /// One-line description for UI tooltip.
    pub description: String,
    /// Vendor / project homepage URL.
    pub homepage_url: String,
    /// Sign-up / pricing URL.
    pub signup_url: String,
    /// Docs URL.
    pub docs_url: String,
    /// Which QueryTypes this provider supports.
    pub supported_query_types: Vec<QueryType>,
    /// Daily / monthly free-tier quota text (for UI hint).
    pub quota_hint: String,
    /// Whether this provider requires a paid plan.
    pub requires_paid: bool,
    /// Integration schema describing which credential fields this
    /// provider expects and how to test them.
    ///
    /// `None` is allowed during the rollout (Phase 1) so existing
    /// providers keep compiling before Phase 5 fills them in;
    /// `Some(_)` makes the provider show up in the Integrations
    /// Settings tab with a fully-rendered form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration_schema: Option<golish_integrations::IntegrationSchema>,
}

/// Uniform record produced by every provider.
///
/// `fields` keys are normalized to match what `store_organization_update`
/// expects so the writer can route values to the right column without
/// knowing which provider produced them.
///
/// Common keys (not exhaustive):
/// - `domain` · push into `organizations.domains`
/// - `cidr`   · push into `organizations.ip_ranges`
/// - `asn`    · push into `organizations.asns`
/// - `cert`   · push into `organizations.certificates`
/// - `email`  · push into `organizations.email_domains`
/// - `github_org` · push into `organizations.github_orgs`
/// - `contact_name` · push into `organizations.contacts`
/// - `subsidiary` · push into `organizations.subsidiaries`
/// - `cloud_asset` · push into `organizations.cloud_assets`
/// - `target` · trigger new `target_add`
/// - `webserver` / `cms` / `cdn` / `os` · push into `fingerprints.*`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRecord {
    pub provider: String,
    pub query_type: QueryType,
    pub fields: HashMap<String, String>,
    /// Original raw response for evidence / audit. Provider impls fill it
    /// even on success so downstream consumers (graphiti, evidence ledger)
    /// can reference it.
    pub raw: serde_json::Value,
    pub fetched_at: DateTime<Utc>,
}

impl ProviderRecord {
    pub fn new(
        provider: impl Into<String>,
        query_type: QueryType,
        fields: HashMap<String, String>,
        raw: serde_json::Value,
    ) -> Self {
        Self {
            provider: provider.into(),
            query_type,
            fields,
            raw,
            fetched_at: Utc::now(),
        }
    }
}

/// Connection test outcome, surfaced to the Settings "Test Connection" button.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ConnectionStatus {
    /// Key works, quota info filled if the provider exposes it.
    Ok {
        message: String,
        quota_remaining: Option<u64>,
        quota_total: Option<u64>,
    },
    /// Key rejected (401 / 403 / banned).
    AuthFailed { message: String },
    /// Quota exhausted (key valid but cannot query).
    QuotaExhausted { message: String },
    /// Network-level failure (DNS / TLS / 5xx).
    NetworkError { message: String },
}

#[cfg(test)]
mod semantic_literal_tests {
    use super::*;

    #[test]
    fn provider_literal_compilers_escape_quotes_slashes_and_operators() {
        let input = "Acme\\\" OR domain=\\\"evil.test: 中文";
        for query in [
            crate::fofa::compile_semantic_query(QueryType::Org, input).unwrap(),
            crate::hunter::compile_semantic_query(QueryType::Org, input).unwrap(),
            crate::shodan::compile_semantic_query(QueryType::Org, input).unwrap(),
            crate::quake::compile_semantic_query(QueryType::Org, input).unwrap(),
        ] {
            assert!(query.contains("\\\\\\\""));
            assert!(query.contains("OR domain="));
            assert!(!query.contains(" OR domain=\""));
            assert!(query.contains("中文"));
        }
    }
}
