//! 0.zone (零零信安) provider implementation.
//!
//! API: <https://0.zone/api/data/>
//! Docs: <https://0.zone/plug-in-unit>
//! Reference impl: <https://github.com/lemonlove7/0_zone/blob/main/zone_api.py>
//!
//! ## Supported QueryTypes
//!
//! - [`QueryType::Site`] · 信息系统（ip + url + title + status + group + operator + cms）
//! - [`QueryType::Domain`] · 子域名
//! - [`QueryType::Email`] · 邮箱
//! - [`QueryType::Apk`] · 移动端应用
//! - [`QueryType::Code`] · 代码/文档泄漏
//! - [`QueryType::Member`] · 人员
//! - [`QueryType::Org`] · 企业画像
//!
//! ## Rate limit
//!
//! Free tier: 250 queries/day · 2 req/s. The provider holds a single
//! [`RateLimiter`] enforcing the 2-req/s ceiling.

mod client;
mod mapper;
mod types;

#[cfg(test)]
pub use types::{ApkEntry, CodeEntry, DomainEntry, EmailEntry, MemberEntry, SiteEntry};

use async_trait::async_trait;
use tracing::{debug, warn};

use crate::error::{IntelError, IntelResult};
use crate::shared::RateLimiter;
use crate::types::{ConnectionStatus, ProviderMeta, ProviderRecord, QueryType};
use crate::IntelProvider;

const PROVIDER_ID: &str = "0.zone";

/// The 0.zone (零零信安) provider.
pub struct ZoneProvider {
    http: reqwest::Client,
    rate_limit: RateLimiter,
}

impl Default for ZoneProvider {
    fn default() -> Self {
        Self {
            http: client::default_http_client(),
            rate_limit: RateLimiter::two_per_second(),
        }
    }
}

impl std::fmt::Debug for ZoneProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZoneProvider").finish_non_exhaustive()
    }
}

impl ZoneProvider {
    /// Construct with a custom HTTP client (e.g. for testing with mock servers).
    pub fn with_http_client(http: reqwest::Client) -> Self {
        Self {
            http,
            rate_limit: RateLimiter::two_per_second(),
        }
    }

    fn query_type_wire(qtype: QueryType) -> IntelResult<&'static str> {
        match qtype {
            QueryType::Site => Ok("site"),
            QueryType::Domain => Ok("domain"),
            QueryType::Email => Ok("email"),
            QueryType::Apk => Ok("apk"),
            QueryType::Code => Ok("code"),
            QueryType::Member => Ok("member"),
            QueryType::Org => Ok("org"),
            other => Err(IntelError::UnsupportedQueryType {
                provider: PROVIDER_ID.into(),
                query_type: other.as_str().into(),
            }),
        }
    }

    fn map_entry(qtype: QueryType, raw: serde_json::Value) -> IntelResult<ProviderRecord> {
        match qtype {
            QueryType::Site => {
                let e: types::SiteEntry = serde_json::from_value(raw.clone())
                    .map_err(|err| IntelError::bad_response(PROVIDER_ID, err.to_string()))?;
                Ok(mapper::map_site(e, raw))
            }
            QueryType::Domain => {
                let e: types::DomainEntry = serde_json::from_value(raw.clone())
                    .map_err(|err| IntelError::bad_response(PROVIDER_ID, err.to_string()))?;
                Ok(mapper::map_domain(e, raw))
            }
            QueryType::Email => {
                let e: types::EmailEntry = serde_json::from_value(raw.clone())
                    .map_err(|err| IntelError::bad_response(PROVIDER_ID, err.to_string()))?;
                Ok(mapper::map_email(e, raw))
            }
            QueryType::Apk => {
                let e: types::ApkEntry = serde_json::from_value(raw.clone())
                    .map_err(|err| IntelError::bad_response(PROVIDER_ID, err.to_string()))?;
                Ok(mapper::map_apk(e, raw))
            }
            QueryType::Code => {
                let e: types::CodeEntry = serde_json::from_value(raw.clone())
                    .map_err(|err| IntelError::bad_response(PROVIDER_ID, err.to_string()))?;
                Ok(mapper::map_code(e, raw))
            }
            QueryType::Member => {
                let e: types::MemberEntry = serde_json::from_value(raw.clone())
                    .map_err(|err| IntelError::bad_response(PROVIDER_ID, err.to_string()))?;
                Ok(mapper::map_member(e, raw))
            }
            QueryType::Org => {
                let e: types::OrgEntry = serde_json::from_value(raw.clone())
                    .map_err(|err| IntelError::bad_response(PROVIDER_ID, err.to_string()))?;
                Ok(mapper::map_org(e, raw))
            }
            other => Err(IntelError::UnsupportedQueryType {
                provider: PROVIDER_ID.into(),
                query_type: other.as_str().into(),
            }),
        }
    }
}

#[async_trait]
impl IntelProvider for ZoneProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            id: PROVIDER_ID.into(),
            display_name: "0.zone（零零信安）".into(),
            description:
                "国内网络空间测绘 + 暗网情报双引擎（7 query_type · 含 group 公司归属反查）".into(),
            homepage_url: "https://0.zone".into(),
            signup_url: "https://0.zone/plug-in-unit".into(),
            docs_url: "https://0.zone/grammarList".into(),
            supported_query_types: vec![
                QueryType::Site,
                QueryType::Domain,
                QueryType::Email,
                QueryType::Apk,
                QueryType::Code,
                QueryType::Member,
                QueryType::Org,
            ],
            quota_hint: "基础会员 ¥98/年 · 250 次/日 · ≤2 req/s".into(),
            requires_paid: true,
            integration_schema: Some(crate::api_key_integration_schema(
                "0.zone（零零信安）",
                "国内网络空间测绘 + 暗网情报双引擎",
                Some("API key from https://0.zone/plug-in-unit profile page"),
                Some("https://0.zone/plug-in-unit"),
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
                reason: "zone_key_id is empty".into(),
            });
        }

        let wire_qtype = Self::query_type_wire(query_type)?;
        self.rate_limit.acquire().await;

        debug!(
            provider = PROVIDER_ID,
            query_type = wire_qtype,
            "issuing 0.zone query"
        );

        let envelope: types::ZoneEnvelope =
            client::post_query(&self.http, query, wire_qtype, 1, key).await?;

        if !envelope.is_ok() {
            return Err(client::classify_envelope_error(&envelope.message));
        }

        let mut records = Vec::with_capacity(envelope.data.len());
        for raw in envelope.data {
            match Self::map_entry(query_type, raw.clone()) {
                Ok(r) => records.push(r),
                Err(e) => {
                    warn!(provider = PROVIDER_ID, error = %e, "skipping malformed entry");
                }
            }
        }

        Ok(records)
    }

    async fn test_connection(&self, key: &str) -> IntelResult<ConnectionStatus> {
        if key.trim().is_empty() {
            return Ok(ConnectionStatus::AuthFailed {
                message: "zone_key_id is empty".into(),
            });
        }
        // Use a cheap query that should always succeed if the key is valid.
        // We pick `query_type=site` with a known dummy query; 0.zone returns
        // 0-row data but `code == 0` if the key is fine.
        self.rate_limit.acquire().await;
        let result: IntelResult<types::ZoneEnvelope> =
            client::post_query(&self.http, "0.zone-test", "site", 1, key).await;

        match result {
            Ok(env) if env.is_ok() => Ok(ConnectionStatus::Ok {
                message: "0.zone API key validated".into(),
                quota_remaining: None, // 0.zone does not expose this in the response
                quota_total: None,
            }),
            Ok(env) => match client::classify_envelope_error(&env.message) {
                IntelError::AuthFailed { .. } => Ok(ConnectionStatus::AuthFailed {
                    message: env.message,
                }),
                IntelError::QuotaExceeded { .. } => Ok(ConnectionStatus::QuotaExhausted {
                    message: env.message,
                }),
                _ => Ok(ConnectionStatus::NetworkError {
                    message: env.message,
                }),
            },
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
    fn zone_provider_metadata_is_sane() {
        let p = ZoneProvider::default();
        assert_eq!(p.id(), "0.zone");
        let m = p.meta();
        assert_eq!(m.id, "0.zone");
        assert_eq!(m.supported_query_types.len(), 7);
        assert!(m.requires_paid);
    }

    #[test]
    fn query_type_wire_supports_seven_known() {
        for qt in [
            QueryType::Site,
            QueryType::Domain,
            QueryType::Email,
            QueryType::Apk,
            QueryType::Code,
            QueryType::Member,
            QueryType::Org,
        ] {
            assert!(
                ZoneProvider::query_type_wire(qt).is_ok(),
                "{qt:?} should be supported"
            );
        }
    }

    #[test]
    fn query_type_wire_rejects_cert() {
        let res = ZoneProvider::query_type_wire(QueryType::Cert);
        assert!(matches!(res, Err(IntelError::UnsupportedQueryType { .. })));
    }

    #[test]
    fn query_type_wire_rejects_old_sensitive_type() {
        let res = ZoneProvider::query_type_wire(QueryType::Sensitive);
        assert!(matches!(res, Err(IntelError::UnsupportedQueryType { .. })));
    }

    #[tokio::test]
    async fn query_rejects_empty_key() {
        let p = ZoneProvider::default();
        let err = p
            .query(QueryType::Site, "example.com", "   ")
            .await
            .unwrap_err();
        assert!(matches!(err, IntelError::InvalidKey { .. }));
    }

    #[tokio::test]
    async fn test_connection_returns_auth_failed_for_empty_key() {
        let p = ZoneProvider::default();
        let status = p.test_connection("").await.unwrap();
        assert!(matches!(status, ConnectionStatus::AuthFailed { .. }));
    }
}
