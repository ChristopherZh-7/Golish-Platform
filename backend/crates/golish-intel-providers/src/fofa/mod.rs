//! FOFA (鹰图) provider implementation.
//!
//! API: <https://fofa.info/api/v1/search/all>
//! Docs: <https://fofa.info/api>
//!
//! ## Supported QueryTypes
//!
//! - [`QueryType::Site`] · host/ip + port + protocol + title + server + cert
//! - [`QueryType::Domain`] · 子域名 / 主域查询（FOFA 语法 `domain="..."`）
//! - [`QueryType::Cert`] · 证书查询（FOFA 语法 `cert="..."`）
//!
//! ## Authentication
//!
//! FOFA v1 requires both an *email* and an *API key*. The vault holds a
//! single string per entry, so we use the canonical format
//! `"<email>|<key>"` and split it inside [`client::split_credentials`].
//! Settings UI must instruct the user to enter their credential in that
//! exact form.
//!
//! ## Rate limit
//!
//! FOFA does not impose a per-second rate limit, but exhausts an F-point
//! daily quota (免费账户每日 100 次). We still pace at 2 req/s defensively
//! to behave well when used in long-running batches.

mod client;
mod mapper;
mod types;

#[cfg(test)]
pub use types::{FofaEnvelope, FofaRow, FofaUserInfo};

use async_trait::async_trait;
use tracing::{debug, warn};

use crate::error::{IntelError, IntelResult};
use crate::shared::RateLimiter;
use crate::types::{ConnectionStatus, ProviderMeta, ProviderRecord, QueryType};
use crate::IntelProvider;

const PROVIDER_ID: &str = "fofa";

/// FOFA (鹰图) provider.
pub struct FofaProvider {
    http: reqwest::Client,
    rate_limit: RateLimiter,
}

impl Default for FofaProvider {
    fn default() -> Self {
        Self {
            http: client::default_http_client(),
            rate_limit: RateLimiter::two_per_second(),
        }
    }
}

impl std::fmt::Debug for FofaProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FofaProvider").finish_non_exhaustive()
    }
}

impl FofaProvider {
    /// Construct with a custom HTTP client (for mocked transports in tests).
    pub fn with_http_client(http: reqwest::Client) -> Self {
        Self {
            http,
            rate_limit: RateLimiter::two_per_second(),
        }
    }

    /// Render the FOFA query string for a given [`QueryType`] + user input.
    ///
    /// Returns `Err` for unsupported types; the caller may translate that
    /// into a typed [`IntelError::UnsupportedQueryType`].
    pub(crate) fn render_query(qtype: QueryType, q: &str) -> IntelResult<String> {
        let q = q.trim();
        if q.is_empty() {
            return Err(IntelError::Other("fofa query string is empty".into()));
        }
        // JSON provider configs may intentionally supply a complete FOFA DSL
        // expression (for example `org="Acme"`). Preserve it verbatim instead
        // of nesting it inside `host="..."` / `cert="..."`. Domain-mode
        // templates pass a raw hostname and still take the typed wrapper below.
        if q.contains('=') {
            return Ok(q.to_string());
        }
        match qtype {
            QueryType::Site => Ok(format!("host=\"{q}\"")),
            QueryType::Domain => Ok(format!("domain=\"{q}\"")),
            QueryType::Cert => Ok(format!("cert=\"{q}\"")),
            other => Err(IntelError::UnsupportedQueryType {
                provider: PROVIDER_ID.into(),
                query_type: other.as_str().into(),
            }),
        }
    }

    fn map_row(qtype: QueryType, raw_row: &[String]) -> Option<ProviderRecord> {
        let row = types::FofaEnvelope::row(raw_row)?;
        let raw_json = serde_json::Value::Array(
            raw_row
                .iter()
                .map(|s| serde_json::Value::String(s.clone()))
                .collect(),
        );
        Some(match qtype {
            QueryType::Site => mapper::map_site(row, raw_json),
            QueryType::Domain => mapper::map_domain(row, raw_json),
            QueryType::Cert => mapper::map_cert(row, raw_json),
            // Unsupported types are filtered out before we get here.
            _ => return None,
        })
    }
}

#[async_trait]
impl IntelProvider for FofaProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            id: PROVIDER_ID.into(),
            display_name: "FOFA（鹰图）".into(),
            description: "白帽汇 FOFA · 国内主流网络空间测绘 · 支持 host/domain/cert 等查询语法"
                .into(),
            homepage_url: "https://fofa.info".into(),
            signup_url: "https://fofa.info/register".into(),
            docs_url: "https://fofa.info/api".into(),
            supported_query_types: vec![QueryType::Site, QueryType::Domain, QueryType::Cert],
            quota_hint: "免费账户每日 100 条；vault key 格式 'email|key'".into(),
            requires_paid: false,
            integration_schema: Some(crate::api_key_integration_schema(
                "FOFA（鹰图）",
                "白帽汇 FOFA · 国内主流网络空间测绘",
                Some("email|api_key (combined, separated by `|`)"),
                Some("https://fofa.info/register"),
            )),
        }
    }

    async fn query(
        &self,
        query_type: QueryType,
        query: &str,
        key: &str,
    ) -> IntelResult<Vec<ProviderRecord>> {
        let (email, api_key) = client::split_credentials(key)?;
        let q = Self::render_query(query_type, query)?;

        self.rate_limit.acquire().await;
        debug!(
            provider = PROVIDER_ID,
            query_type = query_type.as_str(),
            "issuing fofa query"
        );

        let env: types::FofaEnvelope =
            client::search(&self.http, email, api_key, &q, types::FOFA_FIELDS, None).await?;

        if !env.is_ok() {
            let errmsg = env.errmsg.as_deref().unwrap_or("unknown fofa error");
            return Err(client::classify_errmsg(errmsg));
        }

        let mut records = Vec::with_capacity(env.results.len());
        for row in env.results {
            match Self::map_row(query_type, &row) {
                Some(rec) => records.push(rec),
                None => warn!(provider = PROVIDER_ID, "skipping malformed fofa row"),
            }
        }
        Ok(records)
    }

    async fn test_connection(&self, key: &str) -> IntelResult<ConnectionStatus> {
        let (email, api_key) = match client::split_credentials(key) {
            Ok(p) => p,
            Err(IntelError::InvalidKey { reason, .. }) => {
                return Ok(ConnectionStatus::AuthFailed { message: reason });
            }
            Err(e) => return Err(e),
        };

        self.rate_limit.acquire().await;
        let info: IntelResult<types::FofaUserInfo> =
            client::fetch_info(&self.http, email, api_key).await;

        Ok(match info {
            Ok(info) if !info.error => ConnectionStatus::Ok {
                message: info
                    .email
                    .map(|e| format!("fofa key validated for {e}"))
                    .unwrap_or_else(|| "fofa key validated".into()),
                quota_remaining: info.remain_free_point,
                quota_total: info.fofa_point,
            },
            Ok(info) => match client::classify_errmsg(info.errmsg.as_deref().unwrap_or("")) {
                IntelError::QuotaExceeded { reason, .. } => {
                    ConnectionStatus::QuotaExhausted { message: reason }
                }
                IntelError::AuthFailed { reason, .. } => {
                    ConnectionStatus::AuthFailed { message: reason }
                }
                _ => ConnectionStatus::NetworkError {
                    message: info.errmsg.unwrap_or_default(),
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
        let p = FofaProvider::default();
        assert_eq!(p.id(), PROVIDER_ID);
        let m = p.meta();
        assert_eq!(m.id, "fofa");
        assert_eq!(m.supported_query_types.len(), 3);
        assert!(!m.requires_paid);
    }

    #[test]
    fn render_query_supports_three_types() {
        assert_eq!(
            FofaProvider::render_query(QueryType::Site, "example.com").unwrap(),
            "host=\"example.com\""
        );
        assert_eq!(
            FofaProvider::render_query(QueryType::Domain, "example.com").unwrap(),
            "domain=\"example.com\""
        );
        assert_eq!(
            FofaProvider::render_query(QueryType::Cert, "example.com").unwrap(),
            "cert=\"example.com\""
        );
    }

    #[test]
    fn render_query_preserves_explicit_fofa_dsl_from_provider_config() {
        assert_eq!(
            FofaProvider::render_query(QueryType::Site, "org=\"Acme\"").unwrap(),
            "org=\"Acme\""
        );
        assert_eq!(
            FofaProvider::render_query(QueryType::Cert, "cert=\"Acme\"").unwrap(),
            "cert=\"Acme\""
        );
    }

    #[test]
    fn render_query_rejects_unsupported_type() {
        let err = FofaProvider::render_query(QueryType::Email, "x@y.com").unwrap_err();
        assert!(matches!(err, IntelError::UnsupportedQueryType { .. }));
    }

    #[test]
    fn render_query_rejects_empty() {
        let err = FofaProvider::render_query(QueryType::Site, "   ").unwrap_err();
        assert!(matches!(err, IntelError::Other(_)));
    }

    #[tokio::test]
    async fn query_rejects_missing_pipe_in_key() {
        let p = FofaProvider::default();
        let err = p
            .query(QueryType::Site, "example.com", "no-pipe-here")
            .await
            .unwrap_err();
        assert!(matches!(err, IntelError::InvalidKey { .. }));
    }

    #[tokio::test]
    async fn test_connection_returns_auth_failed_for_missing_pipe() {
        let p = FofaProvider::default();
        let status = p.test_connection("only-key-no-email").await.unwrap();
        assert!(matches!(status, ConnectionStatus::AuthFailed { .. }));
    }

    #[test]
    fn map_row_skips_empty_row() {
        let empty: Vec<String> = vec![];
        assert!(FofaProvider::map_row(QueryType::Site, &empty).is_none());
    }

    #[test]
    fn map_row_handles_site_type() {
        let row = vec![
            "example.com".to_string(),
            "1.2.3.4".to_string(),
            "443".to_string(),
            "https".to_string(),
            "example.com".to_string(),
            "Hello".to_string(),
            "nginx".to_string(),
            "US".to_string(),
            "".to_string(),
        ];
        let rec = FofaProvider::map_row(QueryType::Site, &row).unwrap();
        assert_eq!(rec.fields.get("ip").unwrap(), "1.2.3.4");
        assert_eq!(rec.fields.get("title").unwrap(), "Hello");
        assert_eq!(rec.query_type, QueryType::Site);
    }
}
