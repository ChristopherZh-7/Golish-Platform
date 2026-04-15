use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{NewWikiPage, VulnKbLink, VulnKbPoc, WikiPage};

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
