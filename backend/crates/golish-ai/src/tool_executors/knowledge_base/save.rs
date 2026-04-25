//! Write-side knowledge-base verbs:
//!
//! - `write_knowledge`: create / overwrite a wiki page (with frontmatter
//!   parsing, cross-reference indexing, changelog and `index.md` rebuild).
//! - `ingest_cve`: bootstrap a CVE wiki page from the vuln-intel DB.
//! - `save_poc`: persist a proof-of-concept blob into the `pocs` table.

use serde_json::json;

use crate::tool_executors::common::{error_result, extract_string_param, ToolResult};

use super::wiki::{
    extract_wiki_frontmatter, extract_wiki_links, infer_wiki_category, update_wiki_index,
    wiki_base_dir,
};

pub(super) async fn handle_write(
    args: &serde_json::Value,
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> ToolResult {
    let path = match extract_string_param(args, &["path"]) {
        Some(p) if !p.is_empty() => p,
        _ => return error_result("write_knowledge requires a 'path' parameter"),
    };
    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => return error_result("write_knowledge requires a 'content' parameter"),
    };

    let base = wiki_base_dir();
    let full = base.join(&path);
    let is_new = !full.exists();
    if let Some(parent) = full.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return error_result(format!("mkdir failed: {}", e));
        }
    }
    if let Err(e) = tokio::fs::write(&full, &content).await {
        return error_result(format!("write failed: {}", e));
    }

    let cve_id_opt = extract_string_param(args, &["cve_id"]);
    let (title, fm_category, tags, status) = extract_wiki_frontmatter(&content);
    let category = if fm_category == "uncategorized" {
        infer_wiki_category(&path)
    } else {
        fm_category
    };

    // ── Cross-reference extraction ─────────────────────────────────────
    let cross_refs = extract_wiki_links(&content, &path);
    let refs_saved = cross_refs.len();

    if let Some(tracker) = db_tracker {
        let page = golish_db::models::NewWikiPage {
            path: path.clone(),
            title: title.clone(),
            category: category.clone(),
            tags: tags.clone(),
            status: status.clone(),
            content: content.clone(),
        };
        if let Err(e) = golish_db::repo::wiki_kb::upsert_page(tracker.pool(), &page).await {
            tracing::warn!("[kb] DB sync failed for {}: {}", path, e);
        }

        if let Some(ref cve) = cve_id_opt {
            if let Err(e) =
                golish_db::repo::wiki_kb::link_cve_to_wiki(tracker.pool(), cve, &path).await
            {
                tracing::warn!("[kb] CVE link failed for {} -> {}: {}", cve, path, e);
            }
        }

        // Persist cross-references (replace all from this page).
        let _ = golish_db::repo::wiki_kb::delete_refs_from(tracker.pool(), &path).await;
        for (target, ctx) in &cross_refs {
            let _ = golish_db::repo::wiki_kb::upsert_page_ref(tracker.pool(), &path, target, ctx)
                .await;
        }

        // Append changelog entry.
        let action = if is_new { "create" } else { "update" };
        let summary = if is_new {
            format!("Created page: {}", title)
        } else {
            format!("Updated page: {}", title)
        };
        let log_entry = golish_db::models::NewWikiChangelog {
            page_path: path.clone(),
            action: action.to_string(),
            title: title.clone(),
            category: category.clone(),
            actor: "agent".to_string(),
            summary,
        };
        if let Err(e) = golish_db::repo::wiki_kb::add_changelog(tracker.pool(), &log_entry).await {
            tracing::warn!("[kb] changelog write failed: {}", e);
        }
    }

    // ── Append to log.md ───────────────────────────────────────────────
    let log_path = base.join("log.md");
    let action_label = if is_new { "create" } else { "update" };
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M");
    let log_line = format!(
        "\n## [{now}] {action_label} | {title}\n\n- Path: `{path}`\n- Category: {category}\n- Tags: {tags}\n- Status: {status}\n{cve_line}\n",
        title = title,
        path = path,
        category = category,
        tags = if tags.is_empty() { "none".to_string() } else { tags.join(", ") },
        status = status,
        cve_line = cve_id_opt.as_ref().map(|c| format!("- CVE: {c}\n")).unwrap_or_default(),
    );
    if let Err(e) = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .await
    {
        tracing::warn!("[kb] Failed to open log.md for append: {}", e);
    } else {
        use tokio::io::AsyncWriteExt;
        if let Ok(mut f) = tokio::fs::OpenOptions::new().append(true).open(&log_path).await {
            let _ = f.write_all(log_line.as_bytes()).await;
        }
    }

    // ── Update index.md ────────────────────────────────────────────────
    update_wiki_index(&base).await;

    let mut msg = format!("Knowledge page written to {}", path);
    if let Some(ref cve) = cve_id_opt {
        msg.push_str(&format!(" (linked to {})", cve));
    }
    if refs_saved > 0 {
        msg.push_str(&format!(", {} cross-references indexed", refs_saved));
    }

    (
        json!({
            "success": true,
            "path": path,
            "linked_cve": cve_id_opt,
            "cross_references": refs_saved,
            "message": msg,
        }),
        true,
    )
}

pub(super) async fn handle_ingest_cve(
    args: &serde_json::Value,
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> ToolResult {
    let cve_id = match extract_string_param(args, &["cve_id", "cve"]) {
        Some(c) if !c.is_empty() => c,
        _ => return error_result("ingest_cve requires a 'cve_id' parameter"),
    };
    let product = match extract_string_param(args, &["product", "component"]) {
        Some(p) if !p.is_empty() => p,
        _ => return error_result("ingest_cve requires a 'product' parameter"),
    };
    let additional = extract_string_param(args, &["additional_context", "notes"]);

    let mut cve_info = String::new();
    if let Some(tracker) = db_tracker {
        match golish_db::repo::vuln_intel::search_entries(tracker.pool(), &cve_id, 1).await {
            Ok(entries) if !entries.is_empty() => {
                let e = &entries[0];
                cve_info = format!(
                    "Title: {}\nSeverity: {}\nCVSS: {}\nPublished: {}\nDescription: {}\nAffected: {}\nReferences: {}",
                    e.title,
                    e.sev,
                    e.cvss_score.map_or("N/A".to_string(), |s| s.to_string()),
                    e.published,
                    e.description,
                    e.affected_products,
                    e.refs,
                );
            }
            _ => {}
        }
    }

    let slug = product.to_lowercase().replace(' ', "-");
    let path = format!("products/{}/{}.md", slug, cve_id);
    let base = wiki_base_dir();
    let full = base.join(&path);

    if full.exists() {
        let existing = tokio::fs::read_to_string(&full).await.unwrap_or_default();
        if let Some(tracker) = db_tracker {
            let _ =
                golish_db::repo::wiki_kb::link_cve_to_wiki(tracker.pool(), &cve_id, &path).await;
        }
        return (
            json!({
                "exists": true,
                "path": path,
                "content": existing,
                "message": format!("Wiki page already exists at {}. Read and update it with write_knowledge if needed.", path),
            }),
            true,
        );
    }

    let mut page_content = format!(
        "---\ntitle: \"{} — {}\"\ncategory: products\ntags: [{}]\ncves: [{}]\nstatus: draft\n---\n\n# {} — {}\n\n",
        cve_id, product, slug, cve_id, cve_id, product
    );

    if !cve_info.is_empty() {
        page_content.push_str("## Vulnerability Details\n\n");
        page_content.push_str(&cve_info);
        page_content.push_str("\n\n");
    } else {
        page_content.push_str(
            "## Vulnerability Details\n\n> No data in vuln intel DB. Research needed.\n\n",
        );
    }

    page_content.push_str(
        "## Exploitation\n\n<!-- Add exploit method, PoC, and attack chain details here -->\n\n",
    );
    page_content.push_str("## Detection & Mitigation\n\n<!-- How to detect and fix -->\n\n");
    page_content.push_str("## Notes\n\n");

    if let Some(ctx) = additional {
        page_content.push_str(&ctx);
        page_content.push('\n');
    }

    if let Some(parent) = full.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return error_result(format!("mkdir failed: {}", e));
        }
    }
    if let Err(e) = tokio::fs::write(&full, &page_content).await {
        return error_result(format!("write failed: {}", e));
    }

    if let Some(tracker) = db_tracker {
        let page = golish_db::models::NewWikiPage {
            path: path.clone(),
            title: format!("{} — {}", cve_id, product),
            category: "products".to_string(),
            tags: vec![slug.clone(), cve_id.clone()],
            status: "draft".to_string(),
            content: page_content.clone(),
        };
        let _ = golish_db::repo::wiki_kb::upsert_page(tracker.pool(), &page).await;
        let _ = golish_db::repo::wiki_kb::link_cve_to_wiki(tracker.pool(), &cve_id, &path).await;
    }

    (
        json!({
            "success": true,
            "path": path,
            "message": format!("Created wiki page for {} at {}. Edit with write_knowledge to add exploit details.", cve_id, path),
        }),
        true,
    )
}

pub(super) async fn handle_save_poc(
    args: &serde_json::Value,
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> ToolResult {
    let cve_id = match extract_string_param(args, &["cve_id"]) {
        Some(c) if !c.is_empty() => c,
        _ => return error_result("save_poc requires a 'cve_id' parameter"),
    };
    let name = match extract_string_param(args, &["name"]) {
        Some(n) if !n.is_empty() => n,
        _ => return error_result("save_poc requires a 'name' parameter"),
    };
    let poc_type = extract_string_param(args, &["poc_type"]).unwrap_or_else(|| "script".to_string());
    let language =
        extract_string_param(args, &["language"]).unwrap_or_else(|| "python".to_string());
    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => return error_result("save_poc requires a 'content' parameter"),
    };

    let source = extract_string_param(args, &["source"]).unwrap_or_else(|| "manual".to_string());
    let source_url = extract_string_param(args, &["source_url"]).unwrap_or_default();
    let severity =
        extract_string_param(args, &["severity"]).unwrap_or_else(|| "unknown".to_string());
    let description = extract_string_param(args, &["description"]).unwrap_or_default();
    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if let Some(tracker) = db_tracker {
        match golish_db::repo::wiki_kb::upsert_poc_full(
            tracker.pool(),
            &cve_id,
            &name,
            &poc_type,
            &language,
            &content,
            &source,
            &source_url,
            &severity,
            &description,
            &tags,
        )
        .await
        {
            Ok(poc) => (
                json!({
                    "success": true,
                    "poc_id": poc.id.to_string(),
                    "source": source,
                    "severity": severity,
                    "message": format!("PoC '{}' saved for {} (source: {}, severity: {})", name, cve_id, source, severity),
                }),
                true,
            ),
            Err(e) => error_result(format!("Failed to save PoC: {}", e)),
        }
    } else {
        error_result("Database not available")
    }
}
