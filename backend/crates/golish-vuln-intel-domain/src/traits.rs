//! Port traits defining I/O boundaries for the vuln-intel domain.

use crate::types::{VulnEntry, VulnFeed};

/// Database port for persisting vulnerability intelligence data.
pub trait VulnIntelRepo: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    fn ensure_default_feeds(
        &self,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    fn upsert_entries(
        &self,
        entries: &[VulnEntry],
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    fn list_feeds(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<VulnFeed>, Self::Error>> + Send;
}
