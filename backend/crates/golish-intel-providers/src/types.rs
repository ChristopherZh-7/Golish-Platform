//! Shared data types for the intel-providers crate.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Query category. Each provider declares which subset of these it supports.
///
/// The names roughly map to the `query_type` parameter of various ASM
/// platforms (0.zone uses `site` / `domain` / `email` / `apk` / `sensitive`
/// / `code` / `member`; FOFA / Quake use similar but distinct vocabularies).
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
            Self::Cert => "cert",
            Self::Asn => "asn",
            Self::Cidr => "cidr",
        }
    }
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
