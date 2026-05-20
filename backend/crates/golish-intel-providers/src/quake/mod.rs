//! 360 Quake provider · **stub for Phase 4 T4.3**.
//!
//! API: <https://quake.360.net/quake/#/help>

use async_trait::async_trait;

use crate::error::{IntelError, IntelResult};
use crate::types::{ConnectionStatus, ProviderMeta, ProviderRecord, QueryType};
use crate::IntelProvider;

#[derive(Debug, Clone, Default)]
pub struct QuakeProvider;

#[async_trait]
impl IntelProvider for QuakeProvider {
    fn id(&self) -> &str {
        "quake"
    }

    fn meta(&self) -> ProviderMeta {
        ProviderMeta {
            id: "quake".into(),
            display_name: "360 Quake".into(),
            description: "360 网络空间测绘 · 国内三大测绘平台之一".into(),
            homepage_url: "https://quake.360.net".into(),
            signup_url: "https://quake.360.net/quake/#/login".into(),
            docs_url: "https://quake.360.net/quake/#/help".into(),
            supported_query_types: vec![QueryType::Site, QueryType::Domain, QueryType::Cert],
            quota_hint: "免费账户每月有限".into(),
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
            "QuakeProvider not implemented yet (Phase 4 stub)".into(),
        ))
    }

    async fn test_connection(&self, _key: &str) -> IntelResult<ConnectionStatus> {
        Err(IntelError::Other(
            "QuakeProvider not implemented yet (Phase 4 stub)".into(),
        ))
    }
}
