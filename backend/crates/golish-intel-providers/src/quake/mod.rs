//! 360 Quake provider implementation.
//!
//! API base: <https://quake.360.net/api/v3/>
//! Docs: <https://quake.360.net/quake/#/help>
//!
//! ## Supported QueryTypes
//!
//! - [`QueryType::Site`] · 服务 / 主机维度搜索（Quake DSL · `service:`, `port:`, ...）
//! - [`QueryType::Domain`] · 子域名（`domain:"example.com"`）
//! - [`QueryType::Cert`] · 证书查询（`cert:"example.com"`）
//!
//! ## Authentication
//!
//! Single API token via HTTP header `X-QuakeToken: <key>`. The vault stores
//! the raw token; no special encoding required.
//!
//! ## Rate limit
//!
//! Quake imposes a monthly credit quota (default 3000 for free plans).
//! We additionally pace at 2 req/s to behave well during batch lookups.

mod client;
mod mapper;
mod types;

#[cfg(test)]
pub use types::{QuakeEnvelope, QuakeService, QuakeUserInfo};

use async_trait::async_trait;
use tracing::{debug, warn};

use crate::error::{IntelError, IntelResult};
use crate::shared::RateLimiter;
use crate::types::{ConnectionStatus, ProviderMeta, ProviderRecord, QueryType};
use crate::IntelProvider;

const PROVIDER_ID: &str = "quake";

/// 360 Quake provider.
pub struct QuakeProvider {
    http: reqwest::Client,
    rate_limit: RateLimiter,
}

impl Default for QuakeProvider {
    fn default() -> Self {
        Self {
            http: client::default_http_client(),
            rate_limit: RateLimiter::two_per_second(),
        }
    }
}

impl std::fmt::Debug for QuakeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuakeProvider").finish_non_exhaustive()
    }
}

impl QuakeProvider {
    /// Construct with a custom HTTP client (mocked transport for tests).
    pub fn with_http_client(http: reqwest::Client) -> Self {
        Self {
            http,
            rate_limit: RateLimiter::two_per_second(),
        }
    }

    /// Render a Quake DSL query string for a given [`QueryType`] + user input.
    pub(crate) fn render_query(qtype: QueryType, q: &str) -> IntelResult<String> {
        let q = q.trim();
        if q.is_empty() {
            return Err(IntelError::Other("quake query string is empty".into()));
        }
        match qtype {
            QueryType::Site => Ok(format!("host: \"{q}\"")),
            QueryType::Domain => Ok(format!("domain: \"{q}\"")),
            QueryType::Cert => Ok(format!("cert: \"{q}\"")),
            other => Err(IntelError::UnsupportedQueryType {
                provider: PROVIDER_ID.into(),
                query_type: other.as_str().into(),
            }),
        }
    }

    fn map_one(
        qtype: QueryType,
        svc: types::QuakeService,
        raw: serde_json::Value,
    ) -> ProviderRecord {
        match qtype {
            QueryType::Site => mapper::map_site(svc, raw),
            QueryType::Domain => mapper::map_domain(svc, raw),
            QueryType::Cert => mapper::map_cert(svc, raw),
            // Unreachable — render_query rejects everything else upstream.
            _ => mapper::map_site(svc, raw),
        }
    }
}

#[async_trait]
impl IntelProvider for QuakeProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            id: PROVIDER_ID.into(),
            display_name: "360 Quake".into(),
            description: "360 网络空间测绘 · 国内三大测绘平台之一 · 服务+主机维度".into(),
            homepage_url: "https://quake.360.net".into(),
            signup_url: "https://quake.360.net/quake/#/login".into(),
            docs_url: "https://quake.360.net/quake/#/help".into(),
            supported_query_types: vec![QueryType::Site, QueryType::Domain, QueryType::Cert],
            quota_hint: "免费账户每月 3000 条 · 单值 API token".into(),
            requires_paid: false,
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
                reason: "X-QuakeToken is empty".into(),
            });
        }
        let q = Self::render_query(query_type, query)?;

        self.rate_limit.acquire().await;
        debug!(
            provider = PROVIDER_ID,
            query_type = query_type.as_str(),
            "issuing quake_service query"
        );

        let env: types::QuakeEnvelope<Vec<serde_json::Value>> =
            client::search(&self.http, key.trim(), &q, None).await?;

        if !env.is_ok() {
            return Err(client::classify_envelope(env.code, &env.message));
        }

        let data = env.data.unwrap_or_default();
        let mut records = Vec::with_capacity(data.len());
        for raw in data {
            let svc: types::QuakeService = match serde_json::from_value(raw.clone()) {
                Ok(s) => s,
                Err(e) => {
                    warn!(provider = PROVIDER_ID, error = %e, "skipping malformed quake record");
                    continue;
                }
            };
            if svc.ip.is_none() && svc.domain.is_none() && svc.hostname.is_none() {
                warn!(provider = PROVIDER_ID, "skipping empty quake record");
                continue;
            }
            records.push(Self::map_one(query_type, svc, raw));
        }
        Ok(records)
    }

    async fn test_connection(&self, key: &str) -> IntelResult<ConnectionStatus> {
        if key.trim().is_empty() {
            return Ok(ConnectionStatus::AuthFailed {
                message: "X-QuakeToken is empty".into(),
            });
        }

        self.rate_limit.acquire().await;
        let env: IntelResult<types::QuakeEnvelope<types::QuakeUserInfo>> =
            client::fetch_user_info(&self.http, key.trim()).await;

        Ok(match env {
            Ok(env) if env.is_ok() => {
                let info = env.data.unwrap_or_default();
                let username = info
                    .user
                    .as_ref()
                    .and_then(|u| u.username.clone())
                    .unwrap_or_else(|| info.id.clone().unwrap_or_else(|| "quake user".into()));
                ConnectionStatus::Ok {
                    message: format!("quake key validated for {username}"),
                    quota_remaining: info.month_remaining_credit.or(info.remaining_credit),
                    quota_total: info.month_max_credit,
                }
            }
            Ok(env) => match client::classify_envelope(env.code, &env.message) {
                IntelError::QuotaExceeded { reason, .. } => {
                    ConnectionStatus::QuotaExhausted { message: reason }
                }
                IntelError::AuthFailed { reason, .. } => {
                    ConnectionStatus::AuthFailed { message: reason }
                }
                _ => ConnectionStatus::NetworkError {
                    message: env.message,
                },
            },
            Err(IntelError::AuthFailed { reason, .. }) => {
                ConnectionStatus::AuthFailed { message: reason }
            }
            Err(e) => ConnectionStatus::NetworkError {
                message: e.to_string(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_sane() {
        let p = QuakeProvider::default();
        assert_eq!(p.id(), PROVIDER_ID);
        let m = p.meta();
        assert_eq!(m.id, "quake");
        assert_eq!(m.supported_query_types.len(), 3);
        assert!(!m.requires_paid);
    }

    #[test]
    fn render_query_emits_dsl_syntax() {
        assert_eq!(
            QuakeProvider::render_query(QueryType::Site, "1.2.3.4").unwrap(),
            "host: \"1.2.3.4\""
        );
        assert_eq!(
            QuakeProvider::render_query(QueryType::Domain, "example.com").unwrap(),
            "domain: \"example.com\""
        );
        assert_eq!(
            QuakeProvider::render_query(QueryType::Cert, "example.com").unwrap(),
            "cert: \"example.com\""
        );
    }

    #[test]
    fn render_query_rejects_unsupported() {
        let err = QuakeProvider::render_query(QueryType::Email, "x").unwrap_err();
        assert!(matches!(err, IntelError::UnsupportedQueryType { .. }));
    }

    #[test]
    fn render_query_rejects_empty() {
        let err = QuakeProvider::render_query(QueryType::Site, "   ").unwrap_err();
        assert!(matches!(err, IntelError::Other(_)));
    }

    #[tokio::test]
    async fn query_rejects_empty_token() {
        let p = QuakeProvider::default();
        let err = p.query(QueryType::Site, "1.2.3.4", "  ").await.unwrap_err();
        assert!(matches!(err, IntelError::InvalidKey { .. }));
    }

    #[tokio::test]
    async fn test_connection_auth_failed_for_empty_key() {
        let p = QuakeProvider::default();
        let status = p.test_connection("").await.unwrap();
        assert!(matches!(status, ConnectionStatus::AuthFailed { .. }));
    }
}
