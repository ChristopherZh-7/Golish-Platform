//! 奇安信 Hunter provider.
//!
//! API:   `https://hunter.qianxin.com/openApi/search`
//! Docs:  <https://hunter.qianxin.com/home/helpCenter>
//!
//! Hunter authenticates with a single `api-key` token (no email). The
//! `search` query parameter must be **URL-safe** base64 (not standard
//! base64) — see [`client::encode_query`].
//!
//! ## Supported QueryTypes
//!
//! - [`QueryType::Site`] · primary endpoint covering host/ip/port + web
//!   metadata + company (organization_name) field.
//!
//! Additional types (Domain / Cert) are best expressed via the Hunter
//! query DSL inside the existing Site endpoint, so we don't surface them
//! separately yet.

mod client;
mod mapper;
mod types;

#[cfg(test)]
pub use types::{HunterComponent, HunterData, HunterEnvelope, HunterRow};

use async_trait::async_trait;
use tracing::{debug, warn};

use crate::error::{IntelError, IntelResult};
use crate::shared::RateLimiter;
use crate::types::{ConnectionStatus, ProviderMeta, ProviderRecord, QueryType};
use crate::IntelProvider;

const PROVIDER_ID: &str = "hunter";

/// 奇安信 Hunter provider.
pub struct HunterProvider {
    http: reqwest::Client,
    rate_limit: RateLimiter,
}

impl Default for HunterProvider {
    fn default() -> Self {
        Self {
            http: client::default_http_client(),
            rate_limit: RateLimiter::two_per_second(),
        }
    }
}

impl std::fmt::Debug for HunterProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HunterProvider").finish_non_exhaustive()
    }
}

impl HunterProvider {
    pub fn with_http_client(http: reqwest::Client) -> Self {
        Self {
            http,
            rate_limit: RateLimiter::two_per_second(),
        }
    }

    /// Render the Hunter DSL query for the given [`QueryType`].
    pub(crate) fn render_query(qtype: QueryType, q: &str) -> IntelResult<String> {
        let q = q.trim();
        if q.is_empty() {
            return Err(IntelError::Other("hunter query string is empty".into()));
        }
        // Hunter DSL examples:
        //   ip="1.1.1.1"
        //   domain="example.com"
        //   web.title="北京"
        // We map QueryType to the most natural DSL clause.
        match qtype {
            // For Site, take the user's input verbatim. If it doesn't look
            // like DSL (no `=` sign), default to ip="..." or domain="...".
            QueryType::Site => Ok(if q.contains('=') {
                q.to_string()
            } else if q.parse::<std::net::IpAddr>().is_ok() {
                format!("ip=\"{q}\"")
            } else {
                format!("domain=\"{q}\"")
            }),
            QueryType::Domain => Ok(format!("domain=\"{q}\"")),
            QueryType::Cert => Ok(format!("cert=\"{q}\"")),
            QueryType::Asn => Ok(format!("asn=\"{q}\"")),
            QueryType::Cidr => Ok(format!("ip=\"{q}\"")),
            other => Err(IntelError::UnsupportedQueryType {
                provider: PROVIDER_ID.into(),
                query_type: other.as_str().into(),
            }),
        }
    }
}

/// Compile a host-selected semantic literal. Unlike the legacy `Site` path,
/// this function never treats `=` as permission to pass provider DSL through.
pub fn compile_semantic_query(qtype: QueryType, value: &str) -> IntelResult<String> {
    let value = crate::types::escape_provider_literal(value)?;
    let field = match qtype {
        QueryType::Site => "ip",
        QueryType::Domain => "domain",
        QueryType::Cert => "cert",
        QueryType::Asn => "asn",
        QueryType::Cidr => "ip",
        QueryType::Org => "org.name",
        other => {
            return Err(IntelError::UnsupportedQueryType {
                provider: PROVIDER_ID.into(),
                query_type: other.as_str().into(),
            });
        }
    };
    Ok(format!("{field}=\"{value}\""))
}

#[async_trait]
impl IntelProvider for HunterProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            id: PROVIDER_ID.into(),
            display_name: "奇安信 Hunter".into(),
            description: "奇安信网络空间测绘 · 国内主流 ASM 平台之一".into(),
            homepage_url: "https://hunter.qianxin.com".into(),
            signup_url: "https://hunter.qianxin.com/home/userInfo".into(),
            docs_url: "https://hunter.qianxin.com/home/helpCenter".into(),
            supported_query_types: vec![
                QueryType::Site,
                QueryType::Domain,
                QueryType::Cert,
                QueryType::Asn,
                QueryType::Cidr,
            ],
            quota_hint: "每日 / 每月免费配额 · ≤2 req/s 防护".into(),
            requires_paid: false,
            integration_schema: Some(crate::api_key_integration_schema(
                "Hunter（奇安信）",
                "奇安信 Hunter · 国内主流网络空间测绘",
                Some("Hunter API key from hunter.qianxin.com"),
                Some("https://hunter.qianxin.com/home/userInfo"),
            )),
        }
    }

    async fn query(
        &self,
        query_type: QueryType,
        query: &str,
        key: &str,
    ) -> IntelResult<Vec<ProviderRecord>> {
        if key.trim().is_empty() {
            return Err(IntelError::InvalidKey {
                provider: PROVIDER_ID.into(),
                reason: "api-key is empty".into(),
            });
        }
        let rendered = Self::render_query(query_type, query)?;
        self.rate_limit.acquire().await;
        debug!(provider = PROVIDER_ID, ?query_type, "issuing hunter query");

        // is_web=3 → all assets (web + non-web). Use 1 to restrict to web.
        let envelope: types::HunterEnvelope =
            client::search(&self.http, key.trim(), &rendered, 3, None).await?;

        if !envelope.is_ok() {
            return Err(client::classify_code_error(
                envelope.code,
                envelope.error_msg(),
            ));
        }
        let data = envelope.data.unwrap_or_default();
        let mut records = Vec::with_capacity(data.arr.len());
        let raw_total = serde_json::json!({ "total": data.total });
        for row in data.arr {
            let raw = serde_json::to_value(&raw_total).unwrap_or(serde_json::Value::Null);
            let mapped = mapper::map_site(row, raw);
            if mapped.fields.is_empty() {
                warn!(provider = PROVIDER_ID, "skipping empty Hunter row");
                continue;
            }
            records.push(mapped);
        }
        Ok(records)
    }

    async fn test_connection(&self, key: &str) -> IntelResult<ConnectionStatus> {
        if key.trim().is_empty() {
            return Ok(ConnectionStatus::AuthFailed {
                message: "api-key is empty".into(),
            });
        }
        // Cheap probe: search a fixed harmless query, page_size=1.
        self.rate_limit.acquire().await;
        // Hunter rejects very small page sizes; 10 is the smallest observed
        // legal value and still cheap enough for a connection probe.
        let result: IntelResult<types::HunterEnvelope> =
            client::search(&self.http, key.trim(), "ip=\"1.1.1.1\"", 3, Some(10)).await;
        match result {
            Ok(env) if env.is_ok() => {
                let data = env.data.unwrap_or_default();
                let remaining = data.rest_quota.and_then(|s| s.parse::<u64>().ok());
                Ok(ConnectionStatus::Ok {
                    message: "Hunter api-key validated".into(),
                    quota_remaining: remaining,
                    quota_total: None,
                })
            }
            Ok(env) => {
                let msg = env.error_msg().to_string();
                match client::classify_code_error(env.code, &msg) {
                    IntelError::AuthFailed { .. } => {
                        Ok(ConnectionStatus::AuthFailed { message: msg })
                    }
                    IntelError::QuotaExceeded { .. } => {
                        Ok(ConnectionStatus::QuotaExhausted { message: msg })
                    }
                    _ => Ok(ConnectionStatus::NetworkError { message: msg }),
                }
            }
            Err(e) => Ok(ConnectionStatus::NetworkError {
                message: e.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunter_provider_metadata_is_sane() {
        let p = HunterProvider::default();
        assert_eq!(p.id(), "hunter");
        let m = p.meta();
        assert_eq!(m.id, "hunter");
        assert!(!m.supported_query_types.is_empty());
        assert!(!m.requires_paid);
    }

    #[test]
    fn render_query_passes_dsl_through() {
        let rendered = HunterProvider::render_query(QueryType::Site, "web.title=\"x\"").unwrap();
        assert_eq!(rendered, "web.title=\"x\"");
    }

    #[test]
    fn render_query_wraps_bare_ip() {
        let rendered = HunterProvider::render_query(QueryType::Site, "1.2.3.4").unwrap();
        assert_eq!(rendered, "ip=\"1.2.3.4\"");
    }

    #[test]
    fn render_query_wraps_bare_domain() {
        let rendered = HunterProvider::render_query(QueryType::Site, "example.com").unwrap();
        assert_eq!(rendered, "domain=\"example.com\"");
    }

    #[test]
    fn render_query_rejects_empty() {
        let err = HunterProvider::render_query(QueryType::Site, "   ").unwrap_err();
        assert!(matches!(err, IntelError::Other(_)));
    }

    #[test]
    fn render_query_rejects_unsupported_type() {
        let err = HunterProvider::render_query(QueryType::Email, "x@y").unwrap_err();
        assert!(matches!(err, IntelError::UnsupportedQueryType { .. }));
    }

    #[tokio::test]
    async fn query_rejects_empty_key() {
        let p = HunterProvider::default();
        let err = p
            .query(QueryType::Site, "ip=\"1.1.1.1\"", " ")
            .await
            .unwrap_err();
        assert!(matches!(err, IntelError::InvalidKey { .. }));
    }

    #[tokio::test]
    async fn test_connection_auth_failed_on_empty_key() {
        let p = HunterProvider::default();
        let status = p.test_connection("").await.unwrap();
        assert!(matches!(status, ConnectionStatus::AuthFailed { .. }));
    }
}
