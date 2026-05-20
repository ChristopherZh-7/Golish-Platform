//! 奇安信 Hunter provider · **stub for Phase 4 T4.4**.
//!
//! API: <https://hunter.qianxin.com/home/helpCenter>

use async_trait::async_trait;

use crate::error::{IntelError, IntelResult};
use crate::types::{ConnectionStatus, ProviderMeta, ProviderRecord, QueryType};
use crate::IntelProvider;

#[derive(Debug, Clone, Default)]
pub struct HunterProvider;

#[async_trait]
impl IntelProvider for HunterProvider {
    fn id(&self) -> &str {
        "hunter"
    }

    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            id: "hunter".into(),
            display_name: "奇安信 Hunter".into(),
            description: "奇安信网络空间测绘".into(),
            homepage_url: "https://hunter.qianxin.com".into(),
            signup_url: "https://hunter.qianxin.com/home/userInfo".into(),
            docs_url: "https://hunter.qianxin.com/home/helpCenter".into(),
            supported_query_types: vec![QueryType::Site, QueryType::Domain],
            quota_hint: "每日免费配额".into(),
            requires_paid: false,
        }
    }

    async fn query(
        &self,
        _query_type: QueryType,
        _query: &str,
        _key: &str,
    ) -> IntelResult<Vec<ProviderRecord>> {
        Err(IntelError::Other(
            "HunterProvider not implemented yet (Phase 4 stub)".into(),
        ))
    }

    async fn test_connection(&self, _key: &str) -> IntelResult<ConnectionStatus> {
        Err(IntelError::Other(
            "HunterProvider not implemented yet (Phase 4 stub)".into(),
        ))
    }
}
