//! `WikiKbPort` — vuln-owned wiki knowledge-base as a service port (S1-2c).
//!
//! The in-proc adapter mirrors `golish_db::repo::wiki_kb` exactly (read +
//! write). It is the ONLY place the consuming agent service reaches the vuln
//! `wiki_kb` repo; it lives under the vuln port domain so the ownership guard
//! treats it as vuln-owned. Remote-ready: all params/returns are serializable
//! (`golish_db::models::*` derive Serde), no pool leaks across the boundary.

// upsert_poc_full mirrors the wide repo signature verbatim (arity from columns).
#![allow(clippy::too_many_arguments)]

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;

use golish_db::models::{
    CvePocSummary, NewWikiChangelog, NewWikiPage, VulnKbLink, VulnKbPoc, WikiChangelog, WikiPage,
    WikiPageRef,
};

/// Outbound port for the vuln-owned wiki knowledge base (read + write).
#[async_trait]
pub trait WikiKbPort: Send + Sync {
    async fn wiki_upsert_page(&self, page: &NewWikiPage) -> anyhow::Result<WikiPage>;

    async fn wiki_link_cve_to_wiki(
        &self,
        cve_id: &str,
        wiki_path: &str,
    ) -> anyhow::Result<VulnKbLink>;

    async fn wiki_delete_refs_from(&self, source_path: &str) -> anyhow::Result<u64>;

    async fn wiki_upsert_page_ref(
        &self,
        source_path: &str,
        target_path: &str,
        context: &str,
    ) -> anyhow::Result<WikiPageRef>;

    async fn wiki_add_changelog(&self, entry: &NewWikiChangelog) -> anyhow::Result<WikiChangelog>;

    async fn wiki_search_fts(&self, query: &str, limit: i64) -> anyhow::Result<Vec<WikiPage>>;

    async fn wiki_search_by_category(
        &self,
        category: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<WikiPage>>;

    async fn wiki_search_by_tag(&self, tag: &str, limit: i64) -> anyhow::Result<Vec<WikiPage>>;

    async fn wiki_list_cves_with_pocs(&self) -> anyhow::Result<Vec<CvePocSummary>>;

    async fn wiki_list_unresearched_cves(&self, limit: i64) -> anyhow::Result<Vec<CvePocSummary>>;

    async fn wiki_poc_stats(&self) -> anyhow::Result<serde_json::Value>;

    async fn wiki_upsert_poc_full(
        &self,
        cve_id: &str,
        name: &str,
        poc_type: &str,
        language: &str,
        content: &str,
        source: &str,
        source_url: &str,
        severity: &str,
        description: &str,
        tags: &[String],
    ) -> anyhow::Result<VulnKbPoc>;
}

/// In-proc adapter backed by the embedded Postgres pool.
pub struct PgWikiKbAdapter {
    pool: Arc<PgPool>,
}

impl PgWikiKbAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WikiKbPort for PgWikiKbAdapter {
    async fn wiki_upsert_page(&self, page: &NewWikiPage) -> anyhow::Result<WikiPage> {
        Ok(golish_db::repo::wiki_kb::upsert_page(self.pool.as_ref(), page).await?)
    }

    async fn wiki_link_cve_to_wiki(
        &self,
        cve_id: &str,
        wiki_path: &str,
    ) -> anyhow::Result<VulnKbLink> {
        Ok(
            golish_db::repo::wiki_kb::link_cve_to_wiki(self.pool.as_ref(), cve_id, wiki_path)
                .await?,
        )
    }

    async fn wiki_delete_refs_from(&self, source_path: &str) -> anyhow::Result<u64> {
        Ok(golish_db::repo::wiki_kb::delete_refs_from(self.pool.as_ref(), source_path).await?)
    }

    async fn wiki_upsert_page_ref(
        &self,
        source_path: &str,
        target_path: &str,
        context: &str,
    ) -> anyhow::Result<WikiPageRef> {
        Ok(golish_db::repo::wiki_kb::upsert_page_ref(
            self.pool.as_ref(),
            source_path,
            target_path,
            context,
        )
        .await?)
    }

    async fn wiki_add_changelog(&self, entry: &NewWikiChangelog) -> anyhow::Result<WikiChangelog> {
        Ok(golish_db::repo::wiki_kb::add_changelog(self.pool.as_ref(), entry).await?)
    }

    async fn wiki_search_fts(&self, query: &str, limit: i64) -> anyhow::Result<Vec<WikiPage>> {
        Ok(golish_db::repo::wiki_kb::search_fts(self.pool.as_ref(), query, limit).await?)
    }

    async fn wiki_search_by_category(
        &self,
        category: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<WikiPage>> {
        Ok(
            golish_db::repo::wiki_kb::search_by_category(self.pool.as_ref(), category, limit)
                .await?,
        )
    }

    async fn wiki_search_by_tag(&self, tag: &str, limit: i64) -> anyhow::Result<Vec<WikiPage>> {
        Ok(golish_db::repo::wiki_kb::search_by_tag(self.pool.as_ref(), tag, limit).await?)
    }

    async fn wiki_list_cves_with_pocs(&self) -> anyhow::Result<Vec<CvePocSummary>> {
        Ok(golish_db::repo::wiki_kb::list_cves_with_pocs(self.pool.as_ref()).await?)
    }

    async fn wiki_list_unresearched_cves(&self, limit: i64) -> anyhow::Result<Vec<CvePocSummary>> {
        Ok(golish_db::repo::wiki_kb::list_unresearched_cves(self.pool.as_ref(), limit).await?)
    }

    async fn wiki_poc_stats(&self) -> anyhow::Result<serde_json::Value> {
        Ok(golish_db::repo::wiki_kb::poc_stats(self.pool.as_ref()).await?)
    }

    async fn wiki_upsert_poc_full(
        &self,
        cve_id: &str,
        name: &str,
        poc_type: &str,
        language: &str,
        content: &str,
        source: &str,
        source_url: &str,
        severity: &str,
        description: &str,
        tags: &[String],
    ) -> anyhow::Result<VulnKbPoc> {
        Ok(golish_db::repo::wiki_kb::upsert_poc_full(
            self.pool.as_ref(),
            cve_id,
            name,
            poc_type,
            language,
            content,
            source,
            source_url,
            severity,
            description,
            tags,
        )
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiki_kb_port_is_object_safe() {
        fn _assert(_: &dyn WikiKbPort) {}
    }
}
