//! Postgres-backed [`VulnIntelStore`] implementation.
//!
//! Wraps a borrowed `&sqlx::PgPool` and forwards every trait method to the
//! freestanding writers in `types`/`golish_db::repo::wiki_kb`. The
//! freestanding writers are `pub(crate)` so this is the only sanctioned
//! way to persist vuln-intel data from outside the crate.

use std::collections::HashSet;

use async_trait::async_trait;
use sqlx::PgPool;

use crate::store_trait::VulnIntelStore;
use crate::types::{ensure_default_feeds, upsert_entries, VulnEntry};
use crate::VulnIntelResult;

pub struct PgVulnIntelStore<'a> {
    pool: &'a PgPool,
}

impl<'a> PgVulnIntelStore<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<'a> VulnIntelStore for PgVulnIntelStore<'a> {
    async fn ensure_default_feeds(&self) -> VulnIntelResult<()> {
        ensure_default_feeds(self.pool).await
    }

    async fn upsert_entries(&self, entries: &[VulnEntry]) -> VulnIntelResult<()> {
        upsert_entries(self.pool, entries).await
    }

    async fn fetch_existing_poc_identifiers(&self) -> VulnIntelResult<HashSet<String>> {
        let ids: Vec<String> = sqlx::query_scalar::<_, String>(
            "SELECT cve_id FROM vuln_kb_pocs WHERE source = 'nuclei_template'",
        )
        .fetch_all(self.pool)
        .await?;
        Ok(ids.into_iter().collect())
    }

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
    ) -> VulnIntelResult<()> {
        golish_db::repo::wiki_kb::upsert_poc_full(
            self.pool,
            identifier,
            template_name,
            tool,
            format,
            content,
            poc_type,
            source_url,
            severity,
            description,
            tags,
        )
        .await?;
        Ok(())
    }
}
