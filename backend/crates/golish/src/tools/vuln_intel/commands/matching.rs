//! Tauri command: match cached CVE entries to user-defined targets.

use golish_vuln_intel::{EntryRow, VulnEntry};

use crate::error::GolishError;
use crate::state::DbState;

#[tauri::command]
pub async fn intel_match_targets(
    state: tauri::State<'_, DbState>,
    project_path: Option<String>,
) -> Result<Vec<VulnEntry>, GolishError> {
    let pool = state.pool_ready().await?;

    // Scoping guard (AGENTS.md I2): only match against the caller's targets,
    // mirroring `target_list` visibility (incl. legacy global rows).
    let target_rows =
        golish_db::repo::targets::match_rows_legacy(pool, project_path.as_deref()).await?;

    let mut keywords = Vec::new();
    for (name, tags) in &target_rows {
        let lower = name.to_lowercase();
        if lower.len() >= 3 {
            keywords.push(lower);
        }
        if let Some(arr) = tags.as_array() {
            for tag in arr {
                if let Some(s) = tag.as_str() {
                    let lower = s.to_lowercase();
                    if lower.len() >= 3 {
                        keywords.push(lower);
                    }
                }
            }
        }
    }
    keywords.sort();
    keywords.dedup();

    if keywords.is_empty() {
        return Ok(vec![]);
    }

    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT cve_id, title, description, sev, cvss_score, published, source, refs, affected_products \
         FROM vuln_entries ORDER BY published DESC",
    )
    .fetch_all(pool)
    .await?;

    let entries: Vec<VulnEntry> = rows.into_iter().map(VulnEntry::from).collect();
    let matched: Vec<VulnEntry> = entries
        .into_iter()
        .filter(|entry| {
            let text = format!(
                "{} {} {}",
                entry.title,
                entry.description,
                entry.affected_products.join(" ")
            )
            .to_lowercase();
            keywords.iter().any(|kw| text.contains(kw))
        })
        .collect();

    Ok(matched)
}
