//! Shodan provider · **stub for Phase 4 T4.5**.
//!
//! API: <https://developer.shodan.io/api>

use async_trait::async_trait;

use crate::error::{IntelError, IntelResult};
use crate::types::{ConnectionStatus, ProviderMeta, ProviderRecord, QueryType};
use crate::IntelProvider;

#[derive(Debug, Clone, Default)]
pub struct ShodanProvider;

#[async_trait]
impl IntelProvider for ShodanProvider {
    fn id(&self) -> &str {
        "shodan"
    }

    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            id: "shodan".into(),
            display_name: "Shodan".into(),
            description: "国外老牌网络空间测绘".into(),
            homepage_url: "https://www.shodan.io".into(),
            signup_url: "https://account.shodan.io/register".into(),
            docs_url: "https://developer.shodan.io/api".into(),
            supported_query_types: vec![QueryType::Site, QueryType::Domain],
            quota_hint: "$69/月起，免费账户极度受限".into(),
            requires_paid: true,
        }
    }

    async fn query(
        &self,
        _query_type: QueryType,
        _query: &str,
        _key: &str,
    ) -> IntelResult<Vec<ProviderRecord>> {
        Err(IntelError::Other(
            "ShodanProvider not implemented yet (Phase 4 stub)".into(),
        ))
    }

    async fn test_connection(&self, _key: &str) -> IntelResult<ConnectionStatus> {
        Err(IntelError::Other(
            "ShodanProvider not implemented yet (Phase 4 stub)".into(),
        ))
    }
}
