use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{
    CvePocSummary, NewWikiChangelog, NewWikiPage, VulnKbLink, VulnKbPoc, WikiChangelog, WikiPage,
    WikiPageRef, WikiPageSummary,
};

pub async fn upsert_page(pool: &PgPool, page: &NewWikiPage) -> Result<WikiPage> {
    let word_count = page.content.split_whitespace().count() as i32;
    let row = sqlx::query_as::<_, WikiPage>(
        r#"INSERT INTO wiki_pages (path, title, category, tags, status, content, word_count)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (path) DO UPDATE SET
               title = $2, category = $3, tags = $4, status = $5, content = $6,
               word_count = $7, updated_at = NOW()
           RETURNING *"#,
    )
    .bind(&page.path)
    .bind(&page.title)
    .bind(&page.category)
    .bind(&page.tags)
    .bind(&page.status)
    .bind(&page.content)
    .bind(word_count)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn delete_page(pool: &PgPool, path: &str) -> Result<()> {
    sqlx::query("DELETE FROM wiki_pages WHERE path = $1")
        .bind(path)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_pages_by_prefix(pool: &PgPool, prefix: &str) -> Result<u64> {
    let pattern = format!("{}%", prefix);
    let result = sqlx::query("DELETE FROM wiki_pages WHERE path LIKE $1")
        .bind(&pattern)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

pub async fn get_page(pool: &PgPool, path: &str) -> Result<Option<WikiPage>> {
    let row = sqlx::query_as::<_, WikiPage>("SELECT * FROM wiki_pages WHERE path = $1")
        .bind(path)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn search_fts(pool: &PgPool, query: &str, limit: i64) -> Result<Vec<WikiPage>> {
    let rows = sqlx::query_as::<_, WikiPage>(
        r#"SELECT *, ts_rank(
               to_tsvector('english', title || ' ' || content),
               plainto_tsquery('english', $1)
           ) AS rank
           FROM wiki_pages
           WHERE to_tsvector('english', title || ' ' || content)
                 @@ plainto_tsquery('english', $1)
           ORDER BY rank DESC
           LIMIT $2"#,
    )
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn search_by_category(
    pool: &PgPool,
    category: &str,
    limit: i64,
) -> Result<Vec<WikiPage>> {
    let rows = sqlx::query_as::<_, WikiPage>(
        "SELECT * FROM wiki_pages WHERE category = $1 ORDER BY updated_at DESC LIMIT $2",
    )
    .bind(category)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn search_by_tag(pool: &PgPool, tag: &str, limit: i64) -> Result<Vec<WikiPage>> {
    let rows = sqlx::query_as::<_, WikiPage>(
        "SELECT * FROM wiki_pages WHERE $1 = ANY(tags) ORDER BY updated_at DESC LIMIT $2",
    )
    .bind(tag)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_recent(pool: &PgPool, limit: i64) -> Result<Vec<WikiPage>> {
    let rows = sqlx::query_as::<_, WikiPage>(
        "SELECT * FROM wiki_pages ORDER BY updated_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn count_pages(pool: &PgPool) -> Result<i64> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM wiki_pages")
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}

// ============================================================================
// VulnKbLink operations
// ============================================================================

pub async fn link_cve_to_wiki(pool: &PgPool, cve_id: &str, wiki_path: &str) -> Result<VulnKbLink> {
    let row = sqlx::query_as::<_, VulnKbLink>(
        r#"INSERT INTO vuln_kb_links (cve_id, wiki_path)
           VALUES ($1, $2)
           ON CONFLICT (cve_id, wiki_path) DO UPDATE SET cve_id = $1
           RETURNING *"#,
    )
    .bind(cve_id)
    .bind(wiki_path)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn get_links_for_cve(pool: &PgPool, cve_id: &str) -> Result<Vec<VulnKbLink>> {
    let rows = sqlx::query_as::<_, VulnKbLink>(
        "SELECT * FROM vuln_kb_links WHERE cve_id = $1 ORDER BY created_at DESC",
    )
    .bind(cve_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_links_for_path(pool: &PgPool, wiki_path: &str) -> Result<Vec<VulnKbLink>> {
    let rows = sqlx::query_as::<_, VulnKbLink>(
        "SELECT * FROM vuln_kb_links WHERE wiki_path = $1 ORDER BY created_at DESC",
    )
    .bind(wiki_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// ============================================================================
// VulnKbPoc operations
// ============================================================================

pub async fn upsert_poc(
    pool: &PgPool,
    cve_id: &str,
    name: &str,
    poc_type: &str,
    language: &str,
    content: &str,
) -> Result<VulnKbPoc> {
    let row = sqlx::query_as::<_, VulnKbPoc>(
        r#"INSERT INTO vuln_kb_pocs (cve_id, name, poc_type, language, content)
           VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT DO NOTHING
           RETURNING *"#,
    )
    .bind(cve_id)
    .bind(name)
    .bind(poc_type)
    .bind(language)
    .bind(content)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn upsert_poc_full(
    pool: &PgPool,
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
) -> Result<VulnKbPoc> {
    let row = sqlx::query_as::<_, VulnKbPoc>(
        r#"INSERT INTO vuln_kb_pocs
               (cve_id, name, poc_type, language, content, source, source_url, severity, description, tags)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           ON CONFLICT DO NOTHING
           RETURNING *"#,
    )
    .bind(cve_id)
    .bind(name)
    .bind(poc_type)
    .bind(language)
    .bind(content)
    .bind(source)
    .bind(source_url)
    .bind(severity)
    .bind(description)
    .bind(tags)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// List all CVE IDs that have at least one PoC, with counts and research status.
pub async fn list_cves_with_pocs(pool: &PgPool) -> Result<Vec<CvePocSummary>> {
    let rows = sqlx::query_as::<_, CvePocSummary>(
        r#"SELECT
               p.cve_id,
               COUNT(*) as poc_count,
               MAX(p.severity) as max_severity,
               BOOL_OR(p.verified) as any_verified,
               EXISTS(SELECT 1 FROM kb_research_logs r WHERE r.cve_id = p.cve_id) as has_research,
               EXISTS(SELECT 1 FROM vuln_kb_links l WHERE l.cve_id = p.cve_id) as has_wiki
           FROM vuln_kb_pocs p
           GROUP BY p.cve_id
           ORDER BY
               CASE MAX(p.severity)
                   WHEN 'critical' THEN 1
                   WHEN 'high' THEN 2
                   WHEN 'medium' THEN 3
                   WHEN 'low' THEN 4
                   ELSE 5
               END,
               COUNT(*) DESC"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// List CVE IDs that have PoCs but no research yet (priority queue for AI research).
pub async fn list_unresearched_cves(pool: &PgPool, limit: i64) -> Result<Vec<CvePocSummary>> {
    let rows = sqlx::query_as::<_, CvePocSummary>(
        r#"SELECT
               p.cve_id,
               COUNT(*) as poc_count,
               MAX(p.severity) as max_severity,
               BOOL_OR(p.verified) as any_verified,
               FALSE as has_research,
               EXISTS(SELECT 1 FROM vuln_kb_links l WHERE l.cve_id = p.cve_id) as has_wiki
           FROM vuln_kb_pocs p
           WHERE NOT EXISTS(SELECT 1 FROM kb_research_logs r WHERE r.cve_id = p.cve_id)
           GROUP BY p.cve_id
           ORDER BY
               CASE MAX(p.severity)
                   WHEN 'critical' THEN 1
                   WHEN 'high' THEN 2
                   WHEN 'medium' THEN 3
                   WHEN 'low' THEN 4
                   ELSE 5
               END,
               COUNT(*) DESC
           LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Count PoCs by source (nuclei_template, github, manual, etc.)
pub async fn poc_stats(pool: &PgPool) -> Result<serde_json::Value> {
    let by_source: Vec<(String, i64)> = sqlx::query_as(
        "SELECT source, COUNT(*) FROM vuln_kb_pocs GROUP BY source",
    )
    .fetch_all(pool)
    .await?;

    let by_severity: Vec<(String, i64)> = sqlx::query_as(
        "SELECT severity, COUNT(*) FROM vuln_kb_pocs GROUP BY severity",
    )
    .fetch_all(pool)
    .await?;

    let total_cves: (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT cve_id) FROM vuln_kb_pocs",
    )
    .fetch_one(pool)
    .await?;

    let verified: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM vuln_kb_pocs WHERE verified = TRUE",
    )
    .fetch_one(pool)
    .await?;

    Ok(serde_json::json!({
        "by_source": by_source.into_iter().collect::<std::collections::HashMap<_, _>>(),
        "by_severity": by_severity.into_iter().collect::<std::collections::HashMap<_, _>>(),
        "total_cves": total_cves.0,
        "total_verified": verified.0,
    }))
}

pub async fn get_pocs_for_cve(pool: &PgPool, cve_id: &str) -> Result<Vec<VulnKbPoc>> {
    let rows = sqlx::query_as::<_, VulnKbPoc>(
        "SELECT * FROM vuln_kb_pocs WHERE cve_id = $1 ORDER BY created_at DESC",
    )
    .bind(cve_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete_poc(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM vuln_kb_pocs WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ============================================================================
// Wiki Page Cross-References
// ============================================================================

pub async fn upsert_page_ref(
    pool: &PgPool,
    source_path: &str,
    target_path: &str,
    context: &str,
) -> Result<WikiPageRef> {
    let row = sqlx::query_as::<_, WikiPageRef>(
        r#"INSERT INTO wiki_page_refs (source_path, target_path, context)
           VALUES ($1, $2, $3)
           ON CONFLICT (source_path, target_path) DO UPDATE SET context = $3
           RETURNING *"#,
    )
    .bind(source_path)
    .bind(target_path)
    .bind(context)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn delete_refs_from(pool: &PgPool, source_path: &str) -> Result<u64> {
    let result = sqlx::query("DELETE FROM wiki_page_refs WHERE source_path = $1")
        .bind(source_path)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

pub async fn get_outgoing_refs(pool: &PgPool, source_path: &str) -> Result<Vec<WikiPageRef>> {
    let rows = sqlx::query_as::<_, WikiPageRef>(
        "SELECT * FROM wiki_page_refs WHERE source_path = $1 ORDER BY target_path",
    )
    .bind(source_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_backlinks(pool: &PgPool, target_path: &str) -> Result<Vec<WikiPageRef>> {
    let rows = sqlx::query_as::<_, WikiPageRef>(
        "SELECT * FROM wiki_page_refs WHERE target_path = $1 ORDER BY source_path",
    )
    .bind(target_path)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_orphan_pages(pool: &PgPool, limit: i64) -> Result<Vec<WikiPageSummary>> {
    let rows = sqlx::query_as::<_, WikiPageSummary>(
        r#"SELECT w.path, w.title, w.category, w.tags, w.status, w.word_count, w.updated_at
           FROM wiki_pages w
           WHERE NOT EXISTS (
               SELECT 1 FROM wiki_page_refs r WHERE r.target_path = w.path
           )
           AND w.path NOT IN ('SCHEMA.md', 'index.md', 'log.md')
           ORDER BY w.updated_at DESC
           LIMIT $1"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// ============================================================================
// Wiki Changelog
// ============================================================================

pub async fn add_changelog(pool: &PgPool, entry: &NewWikiChangelog) -> Result<WikiChangelog> {
    let row = sqlx::query_as::<_, WikiChangelog>(
        r#"INSERT INTO wiki_changelog (page_path, action, title, category, actor, summary)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING *"#,
    )
    .bind(&entry.page_path)
    .bind(&entry.action)
    .bind(&entry.title)
    .bind(&entry.category)
    .bind(&entry.actor)
    .bind(&entry.summary)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_changelog(pool: &PgPool, limit: i64) -> Result<Vec<WikiChangelog>> {
    let rows = sqlx::query_as::<_, WikiChangelog>(
        "SELECT * FROM wiki_changelog ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_changelog_for_page(
    pool: &PgPool,
    page_path: &str,
    limit: i64,
) -> Result<Vec<WikiChangelog>> {
    let rows = sqlx::query_as::<_, WikiChangelog>(
        "SELECT * FROM wiki_changelog WHERE page_path = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(page_path)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// ============================================================================
// Enhanced Wiki Queries (for frontend grouping/dashboard)
// ============================================================================

pub async fn list_pages_grouped_by_category(pool: &PgPool) -> Result<Vec<WikiPageSummary>> {
    let rows = sqlx::query_as::<_, WikiPageSummary>(
        r#"SELECT path, title, category, tags, status, word_count, updated_at
           FROM wiki_pages
           ORDER BY category, title"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_pages_for_paths(pool: &PgPool, paths: &[String]) -> Result<Vec<WikiPageSummary>> {
    if paths.is_empty() {
        return Ok(vec![]);
    }
    let rows = sqlx::query_as::<_, WikiPageSummary>(
        r#"SELECT path, title, category, tags, status, word_count, updated_at
           FROM wiki_pages
           WHERE path = ANY($1)
           ORDER BY category, title"#,
    )
    .bind(paths)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Find wiki pages that share tags or category with the given CVE's linked pages,
/// but are NOT already linked to that CVE. Used for "suggested" section.
pub async fn suggest_pages_for_cve(
    pool: &PgPool,
    cve_id: &str,
    limit: i64,
) -> Result<Vec<WikiPageSummary>> {
    let rows = sqlx::query_as::<_, WikiPageSummary>(
        r#"WITH linked AS (
               SELECT wiki_path FROM vuln_kb_links WHERE cve_id = $1
           ),
           linked_tags AS (
               SELECT UNNEST(w.tags) AS tag
               FROM wiki_pages w
               JOIN linked l ON l.wiki_path = w.path
           )
           SELECT DISTINCT w.path, w.title, w.category, w.tags, w.status, w.word_count, w.updated_at
           FROM wiki_pages w
           WHERE w.path NOT IN (SELECT wiki_path FROM linked)
             AND w.path NOT IN ('SCHEMA.md', 'index.md', 'log.md')
             AND (
                 EXISTS (SELECT 1 FROM linked_tags lt WHERE lt.tag = ANY(w.tags))
                 OR w.content ILIKE '%' || $1 || '%'
             )
           ORDER BY w.updated_at DESC
           LIMIT $2"#,
    )
    .bind(cve_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn wiki_stats_full(pool: &PgPool) -> Result<serde_json::Value> {
    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM wiki_pages")
        .fetch_one(pool)
        .await?;

    let by_category: Vec<(String, i64)> = sqlx::query_as(
        "SELECT category, COUNT(*) FROM wiki_pages GROUP BY category ORDER BY COUNT(*) DESC",
    )
    .fetch_all(pool)
    .await?;

    let by_status: Vec<(String, i64)> = sqlx::query_as(
        "SELECT status, COUNT(*) FROM wiki_pages GROUP BY status ORDER BY COUNT(*) DESC",
    )
    .fetch_all(pool)
    .await?;

    let total_words: (Option<i64>,) = sqlx::query_as(
        "SELECT SUM(word_count::bigint) FROM wiki_pages",
    )
    .fetch_one(pool)
    .await?;

    let recent_changes: Vec<WikiChangelog> = sqlx::query_as(
        "SELECT * FROM wiki_changelog ORDER BY created_at DESC LIMIT 10",
    )
    .fetch_all(pool)
    .await?;

    let orphan_count: (i64,) = sqlx::query_as(
        r#"SELECT COUNT(*) FROM wiki_pages w
           WHERE NOT EXISTS (SELECT 1 FROM wiki_page_refs r WHERE r.target_path = w.path)
           AND w.path NOT IN ('SCHEMA.md', 'index.md', 'log.md')"#,
    )
    .fetch_one(pool)
    .await?;

    Ok(serde_json::json!({
        "total_pages": total.0,
        "total_words": total_words.0.unwrap_or(0),
        "orphan_count": orphan_count.0,
        "by_category": by_category.into_iter().collect::<std::collections::HashMap<_, _>>(),
        "by_status": by_status.into_iter().collect::<std::collections::HashMap<_, _>>(),
        "recent_changes": recent_changes,
    }))
}
