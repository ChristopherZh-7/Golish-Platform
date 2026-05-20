//! Shodan provider implementation.
//!
//! API:   `https://api.shodan.io/shodan/host/search`
//! Docs:  <https://developer.shodan.io/api>
//!
//! Shodan authenticates via a `key` query string. The DSL query is sent
//! plain (not base64 encoded). Rate limit on the paid plans is 1 req/s
//! (free accounts only get host lookups, no search).
//!
//! ## Supported QueryTypes
//!
//! - [`QueryType::Site`] · maps to `/shodan/host/search` with full banner
//!   surface (ip + port + org + http + ssl + ASN + location).
//! - Domain / Cert / Asn / Cidr are surfaced via the same endpoint by
//!   rewriting the user input into the matching Shodan DSL clause.

mod client;
mod mapper;
mod types;

#[cfg(test)]
pub use types::{ShodanApiInfo, ShodanMatch, ShodanSearchEnvelope};

use async_trait::async_trait;
use tracing::{debug, warn};

use crate::error::{IntelError, IntelResult};
use crate::shared::RateLimiter;
use crate::types::{ConnectionStatus, ProviderMeta, ProviderRecord, QueryType};
use crate::IntelProvider;

const PROVIDER_ID: &str = "shodan";

/// Shodan provider.
pub struct ShodanProvider {
    http: reqwest::Client,
    rate_limit: RateLimiter,
}

impl Default for ShodanProvider {
    fn default() -> Self {
        Self {
            http: client::default_http_client(),
            // Shodan paid plans allow 1 req/s; we pace defensively at 1
            // req every 1000ms.
            rate_limit: RateLimiter::new(std::time::Duration::from_millis(1000)),
        }
    }
}

impl std::fmt::Debug for ShodanProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShodanProvider").finish_non_exhaustive()
    }
}

impl ShodanProvider {
    pub fn with_http_client(http: reqwest::Client) -> Self {
        Self {
            http,
            rate_limit: RateLimiter::new(std::time::Duration::from_millis(1000)),
        }
    }

    /// Render the Shodan DSL query for the given [`QueryType`].
    ///
    /// Shodan DSL examples:
    ///   ip:8.8.8.8
    ///   hostname:example.com
    ///   org:"Acme"
    ///   ssl.cert.subject.cn:example.com
    ///   asn:AS15169
    ///   net:192.0.2.0/24
    pub(crate) fn render_query(qtype: QueryType, q: &str) -> IntelResult<String> {
        let q = q.trim();
        if q.is_empty() {
            return Err(IntelError::Other("shodan query string is empty".into()));
        }
        match qtype {
            QueryType::Site => Ok(if q.contains(':') || q.contains('=') {
                // Looks like Shodan DSL already; pass through.
                q.to_string()
            } else if q.parse::<std::net::IpAddr>().is_ok() {
                format!("ip:{q}")
            } else {
                format!("hostname:{q}")
            }),
            QueryType::Domain => Ok(format!("hostname:{q}")),
            QueryType::Cert => Ok(format!("ssl.cert.subject.cn:{q}")),
            QueryType::Asn => Ok(format!("asn:{q}")),
            QueryType::Cidr => Ok(format!("net:{q}")),
            other => Err(IntelError::UnsupportedQueryType {
                provider: PROVIDER_ID.into(),
                query_type: other.as_str().into(),
            }),
        }
    }
}

#[async_trait]
impl IntelProvider for ShodanProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            id: PROVIDER_ID.into(),
            display_name: "Shodan".into(),
            description: "国外 ASM 元老 · 全球互联网 banner 测绘".into(),
            homepage_url: "https://www.shodan.io".into(),
            signup_url: "https://account.shodan.io/register".into(),
            docs_url: "https://developer.shodan.io/api".into(),
            supported_query_types: vec![
                QueryType::Site,
                QueryType::Domain,
                QueryType::Cert,
                QueryType::Asn,
                QueryType::Cidr,
            ],
            quota_hint: "$69/月起 · 1 req/s · 免费账户仅 host lookup".into(),
            requires_paid: true,
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
                reason: "API key is empty".into(),
            });
        }
        let rendered = Self::render_query(query_type, query)?;
        self.rate_limit.acquire().await;
        debug!(provider = PROVIDER_ID, ?query_type, "issuing shodan query");

        let env: types::ShodanSearchEnvelope =
            client::host_search(&self.http, key.trim(), &rendered).await?;
        if !env.is_ok() {
            return Err(client::classify_message(200, env.error_msg()));
        }
        let mut records = Vec::with_capacity(env.matches.len());
        let raw_meta = serde_json::json!({ "total": env.total });
        for m in env.matches {
            let raw = serde_json::to_value(&raw_meta).unwrap_or(serde_json::Value::Null);
            let mapped = mapper::map_site(m, raw);
            if mapped.fields.is_empty() {
                warn!(provider = PROVIDER_ID, "skipping empty Shodan match");
                continue;
            }
            records.push(mapped);
        }
        Ok(records)
    }

    async fn test_connection(&self, key: &str) -> IntelResult<ConnectionStatus> {
        if key.trim().is_empty() {
            return Ok(ConnectionStatus::AuthFailed {
                message: "API key is empty".into(),
            });
        }
        self.rate_limit.acquire().await;
        let result: IntelResult<types::ShodanApiInfo> =
            client::api_info(&self.http, key.trim()).await;
        match result {
            Ok(info) if info.error.is_none() => Ok(ConnectionStatus::Ok {
                message: format!(
                    "Shodan API key validated · plan: {}",
                    info.plan.as_deref().unwrap_or("unknown")
                ),
                quota_remaining: info.query_credits,
                quota_total: None,
            }),
            Ok(info) => {
                let msg = info.error.unwrap_or_default();
                match client::classify_message(200, &msg) {
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
    fn shodan_provider_metadata_is_sane() {
        let p = ShodanProvider::default();
        assert_eq!(p.id(), "shodan");
        let m = p.meta();
        assert_eq!(m.id, "shodan");
        assert!(m.requires_paid);
        assert!(!m.supported_query_types.is_empty());
    }

    #[test]
    fn render_query_passes_dsl_through() {
        let r = ShodanProvider::render_query(QueryType::Site, "ssl.cert.expired:true").unwrap();
        assert_eq!(r, "ssl.cert.expired:true");
    }

    #[test]
    fn render_query_wraps_bare_ip() {
        let r = ShodanProvider::render_query(QueryType::Site, "1.2.3.4").unwrap();
        assert_eq!(r, "ip:1.2.3.4");
    }

    #[test]
    fn render_query_wraps_bare_hostname() {
        let r = ShodanProvider::render_query(QueryType::Site, "example.com").unwrap();
        assert_eq!(r, "hostname:example.com");
    }

    #[test]
    fn render_query_cert_uses_subject_cn() {
        let r = ShodanProvider::render_query(QueryType::Cert, "example.com").unwrap();
        assert_eq!(r, "ssl.cert.subject.cn:example.com");
    }

    #[test]
    fn render_query_cidr_uses_net() {
        let r = ShodanProvider::render_query(QueryType::Cidr, "10.0.0.0/24").unwrap();
        assert_eq!(r, "net:10.0.0.0/24");
    }

    #[test]
    fn render_query_rejects_email() {
        let err = ShodanProvider::render_query(QueryType::Email, "x@y").unwrap_err();
        assert!(matches!(err, IntelError::UnsupportedQueryType { .. }));
    }

    #[tokio::test]
    async fn query_rejects_empty_key() {
        let p = ShodanProvider::default();
        let err = p
            .query(QueryType::Site, "ip:1.1.1.1", "")
            .await
            .unwrap_err();
        assert!(matches!(err, IntelError::InvalidKey { .. }));
    }

    #[tokio::test]
    async fn test_connection_auth_failed_on_empty_key() {
        let p = ShodanProvider::default();
        let status = p.test_connection("   ").await.unwrap();
        assert!(matches!(status, ConnectionStatus::AuthFailed { .. }));
    }
}
