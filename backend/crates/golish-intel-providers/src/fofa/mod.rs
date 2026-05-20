//! FOFA (鹰图) provider · **stub for Phase 4 T4.2**.
//!
//! API: <https://fofa.info/api>
//! Docs: <https://fofa.info/api>

use async_trait::async_trait;

use crate::error::{IntelError, IntelResult};
use crate::types::{ConnectionStatus, ProviderMeta, ProviderRecord, QueryType};
use crate::IntelProvider;

#[derive(Debug, Clone, Default)]
pub struct FofaProvider;

#[async_trait]
impl IntelProvider for FofaProvider {
    fn id(&self) -> &str {
        "fofa"
    }

    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            id: "fofa".into(),
            display_name: "FOFA（鹰图）".into(),
            description: "白帽汇 FOFA · 国内主流网络空间测绘".into(),
            homepage_url: "https://fofa.info".into(),
            signup_url: "https://fofa.info/register".into(),
            docs_url: "https://fofa.info/api".into(),
            supported_query_types: vec![QueryType::Site, QueryType::Domain, QueryType::Cert],
            quota_hint: "免费账户每日有限".into(),
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
            "FofaProvider not implemented yet (Phase 4 stub)".into(),
        ))
    }

    async fn test_connection(&self, _key: &str) -> IntelResult<ConnectionStatus> {
        Err(IntelError::Other(
            "FofaProvider not implemented yet (Phase 4 stub)".into(),
        ))
    }
}
