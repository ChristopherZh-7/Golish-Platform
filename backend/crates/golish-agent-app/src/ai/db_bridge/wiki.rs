//! Wiki KB domain methods for `GolishDbRepoProvider` (inherent `_impl` layer).
//! Bodies moved verbatim from the original `db_bridge.rs` trait impl; the trait
//! methods in `mod.rs` delegate here.

use super::GolishDbRepoProvider;
use golish_agent_kit::db_traits::*;

impl GolishDbRepoProvider {
    pub(super) async fn wiki_upsert_page_impl(&self, page: &NewWikiPage) -> anyhow::Result<()> {
        let db_page = golish_db::models::NewWikiPage {
            path: page.path.clone(),
            title: page.title.clone(),
            category: page.category.clone(),
            tags: page.tags.clone(),
            status: page.status.clone(),
            content: page.content.clone(),
        };
        self.wiki_kb.wiki_upsert_page(&db_page).await?;
        Ok(())
    }

    pub(super) async fn wiki_link_cve_impl(&self, cve: &str, path: &str) -> anyhow::Result<()> {
        self.wiki_kb.wiki_link_cve_to_wiki(cve, path).await?;
        Ok(())
    }

    pub(super) async fn wiki_delete_refs_from_impl(&self, path: &str) -> anyhow::Result<()> {
        self.wiki_kb.wiki_delete_refs_from(path).await?;
        Ok(())
    }

    pub(super) async fn wiki_upsert_page_ref_impl(
        &self,
        from: &str,
        to: &str,
        ctx: &str,
    ) -> anyhow::Result<()> {
        self.wiki_kb.wiki_upsert_page_ref(from, to, ctx).await?;
        Ok(())
    }

    pub(super) async fn wiki_add_changelog_impl(
        &self,
        entry: &NewWikiChangelog,
    ) -> anyhow::Result<()> {
        let db_entry = golish_db::models::NewWikiChangelog {
            page_path: entry.page_path.clone(),
            action: entry.action.clone(),
            title: entry.title.clone(),
            category: entry.category.clone(),
            actor: entry.actor.clone(),
            summary: entry.summary.clone(),
        };
        self.wiki_kb.wiki_add_changelog(&db_entry).await?;
        Ok(())
    }

    pub(super) async fn wiki_search_fts_impl(
        &self,
        query: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        let results = self.wiki_kb.wiki_search_fts(query, limit).await?;
        Ok(serde_json::to_value(results)?)
    }

    pub(super) async fn wiki_search_by_category_impl(
        &self,
        cat: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        let results = self.wiki_kb.wiki_search_by_category(cat, limit).await?;
        Ok(serde_json::to_value(results)?)
    }

    pub(super) async fn wiki_search_by_tag_impl(
        &self,
        tag: &str,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        let results = self.wiki_kb.wiki_search_by_tag(tag, limit).await?;
        Ok(serde_json::to_value(results)?)
    }

    pub(super) async fn wiki_list_cves_with_pocs_impl(&self) -> anyhow::Result<serde_json::Value> {
        let rows = self.wiki_kb.wiki_list_cves_with_pocs().await?;
        Ok(serde_json::to_value(rows)?)
    }

    pub(super) async fn wiki_list_unresearched_cves_impl(
        &self,
        limit: i64,
    ) -> anyhow::Result<serde_json::Value> {
        let rows = self.wiki_kb.wiki_list_unresearched_cves(limit).await?;
        Ok(serde_json::to_value(rows)?)
    }

    pub(super) async fn wiki_poc_stats_impl(&self) -> anyhow::Result<serde_json::Value> {
        let stats = self.wiki_kb.wiki_poc_stats().await?;
        Ok(stats)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn wiki_upsert_poc_full_impl(
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
    ) -> anyhow::Result<serde_json::Value> {
        let result = self
            .wiki_kb
            .wiki_upsert_poc_full(
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
            .await?;
        Ok(serde_json::to_value(result)?)
    }
}
