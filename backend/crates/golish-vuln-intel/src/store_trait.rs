//! [`VulnIntelStore`] trait — DB-abstraction surface for vuln-intel writes.
//!
//! P2-4 of the refactor roadmap: keep `&sqlx::PgPool` out of the public
//! API of `golish-vuln-intel` so that:
//!
//! 1. The Postgres adapter [`super::pg_adapter::PgVulnIntelStore`] is the
//!    one place that knows about Postgres.
//! 2. Tests (and any future SQLite/embedded backend) can implement
//!    [`VulnIntelStore`] without depending on Postgres.
//!
//! This trait is dyn-compatible (`#[async_trait]`) so callers like
//! [`super::nuclei_discover::discover_all_nuclei`] can take
//! `&dyn VulnIntelStore`.

use std::collections::HashSet;

use async_trait::async_trait;

use crate::types::VulnEntry;
use crate::VulnIntelResult;

/// Storage abstraction for vuln-intel persistence.
///
/// Keep this surface intentionally narrow: only the writes that
/// `golish-vuln-intel` itself emits live here, not generic DB queries.
#[async_trait]
pub trait VulnIntelStore: Send + Sync {
    /// Make sure the canonical default feed rows are present in `vuln_feeds`.
    async fn ensure_default_feeds(&self) -> VulnIntelResult<()>;

    /// Insert/update a batch of CVE entries (idempotent, keyed on `cve_id`).
    async fn upsert_entries(&self, entries: &[VulnEntry]) -> VulnIntelResult<()>;

    /// Snapshot existing nuclei-PoC `identifier` values for de-duplication
    /// during bulk discovery.
    async fn fetch_existing_poc_identifiers(&self) -> VulnIntelResult<HashSet<String>>;

    /// Persist a single nuclei-template PoC record into the wiki KB store.
    #[allow(clippy::too_many_arguments)]
    async fn upsert_nuclei_poc(
        &self,
        identifier: &str,
        template_name: &str,
        tool: &str,
        format: &str,
        content: &str,
        poc_type: &str,
        source_url: &str,
        severity: &str,
        description: &str,
        tags: &[String],
    ) -> VulnIntelResult<()>;
}
