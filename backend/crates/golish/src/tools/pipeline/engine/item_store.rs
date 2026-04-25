use uuid::Uuid;
use golish_pentest::output_store::build_port_json;
use crate::tools::output_parser::ParsedItem;

pub(super) fn extract_hostname(val: &str) -> String {
    if val.starts_with("http://") || val.starts_with("https://") {
        url::Url::parse(val)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
            .unwrap_or_else(|| val.to_string())
    } else {
        val.to_string()
    }
}

/// Returns `Ok(true)` when a brand-new target was created, `Ok(false)` when it already existed.
pub(super) async fn store_target_from_item(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    project_path: Option<&str>,
    parent_id: Option<Uuid>,
) -> Result<bool, String> {
    let hostname = if let Some(h) = item.fields.get("hostname")
        .or_else(|| item.fields.get("host"))
        .or_else(|| item.fields.get("ip"))
    {
        h.clone()
    } else if let Some(url) = item.fields.get("url") {
        extract_hostname(url)
    } else {
        return Err("No hostname/host/ip/url field".to_string());
    };

    let existed = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM targets WHERE value = $1 AND project_path = $2)",
    )
    .bind(&hostname)
    .bind(project_path)
    .fetch_one(pool)
    .await
    .unwrap_or(false);

    crate::tools::targets::db_target_add(pool, &hostname, &hostname, None, project_path, "discovered", parent_id)
        .await?;
    Ok(!existed)
}

/// Returns `Ok(true)` if a port that didn't previously exist was added.
pub(super) async fn store_recon_from_item(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    project_path: Option<&str>,
) -> Result<bool, String> {
    let host_val = item
        .fields
        .get("host")
        .or_else(|| item.fields.get("ip"))
        .or_else(|| item.fields.get("url"))
        .ok_or("No host/ip field")?;

    let hostname = extract_hostname(host_val);
    let target =
        crate::tools::targets::db_target_add(pool, &hostname, &hostname, None, project_path, "discovered", None)
            .await?;
    let target_uuid: Uuid = target.id.parse().map_err(|e: uuid::Error| e.to_string())?;

    let mut update = crate::tools::targets::ReconUpdate::new();
    if let Some(ip) = item.fields.get("ip") {
        update.real_ip = ip.clone();
    }
    if let Some(cdn) = item.fields.get("cdn") {
        update.cdn_waf = cdn.clone();
    }
    if let Some(os) = item.fields.get("os") {
        update.os_info = os.clone();
    }

    if let Some(port_entry) = build_port_json(&item.fields) {
        update.ports = serde_json::json!([port_entry]);
    }

    let is_new_port = if let Some(port_str) = item.fields.get("port") {
        let port_num: i32 = port_str.parse().unwrap_or(0);
        let proto = item.fields.get("protocol").cloned().unwrap_or_else(|| "tcp".to_string());
        !sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM targets WHERE id = $1 AND ports @> $2::jsonb)",
        )
        .bind(target_uuid)
        .bind(serde_json::json!([{"port": port_num, "proto": proto}]))
        .fetch_one(pool)
        .await
        .unwrap_or(false)
    } else {
        false
    };

    if let Some(title) = item.fields.get("title") {
        update.http_title = title.clone();
    }
    if let Some(status) = item.fields.get("status_code").or_else(|| item.fields.get("status")) {
        update.http_status = status.parse().ok();
    }
    if let Some(ws) = item.fields.get("webserver") {
        update.webserver = ws.clone();
    }

    crate::tools::targets::db_target_update_recon_extended(pool, target_uuid, &update).await?;

    let tool_source = item.fields.get("_tool").map(|s| s.as_str()).unwrap_or("httpx");
    crate::tools::output_parser::store_recon_fingerprints(pool, target_uuid, project_path, item, tool_source).await;

    Ok(is_new_port)
}

/// Returns `Ok(true)` if this is a new directory entry.
pub(super) async fn store_dirent_from_item(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    tool_name: &str,
    project_path: Option<&str>,
) -> Result<bool, String> {
    let url = item.fields.get("url").ok_or("No url field")?;

    let existed = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM directory_entries WHERE url = $1 AND project_path = $2)",
    )
    .bind(url)
    .bind(project_path)
    .fetch_one(pool)
    .await
    .unwrap_or(false);

    let status: Option<i32> = item.fields.get("status").and_then(|s| s.parse().ok());
    let size: Option<i32> = item
        .fields
        .get("size")
        .or_else(|| item.fields.get("content_length"))
        .and_then(|s| s.parse().ok());
    let lines: Option<i32> = item.fields.get("lines").and_then(|s| s.parse().ok());
    let words: Option<i32> = item.fields.get("words").and_then(|s| s.parse().ok());

    crate::tools::targets::db_directory_entry_add(
        pool,
        None,
        url,
        status,
        size,
        lines,
        words,
        tool_name,
        project_path,
    )
    .await?;
    Ok(!existed)
}

/// Returns `Ok(true)` if this is a new finding (not a duplicate).
pub(super) async fn store_finding_from_item(
    pool: &sqlx::PgPool,
    item: &ParsedItem,
    tool_name: &str,
    project_path: Option<&str>,
) -> Result<bool, String> {
    let title = item
        .fields
        .get("title")
        .cloned()
        .unwrap_or_else(|| "Untitled Finding".to_string());
    let severity = item
        .fields
        .get("severity")
        .cloned()
        .unwrap_or_else(|| "info".to_string());
    let url = item.fields.get("url").cloned().unwrap_or_default();
    let template = item.fields.get("template").cloned().unwrap_or_default();
    let description = item.fields.get("description").cloned().unwrap_or_default();

    let sev = match severity.to_lowercase().as_str() {
        "critical" => "critical",
        "high" => "high",
        "medium" => "medium",
        "low" => "low",
        _ => "info",
    };

    let result = sqlx::query(
        r#"INSERT INTO findings (title, sev, url, target, description, tool, template, project_path)
           VALUES ($1, $2::severity, $3, $4, $5, $6, $7, $8)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(&title)
    .bind(sev)
    .bind(&url)
    .bind(&url)
    .bind(&description)
    .bind(tool_name)
    .bind(&template)
    .bind(project_path)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(result.rows_affected() > 0)
}

/// Merge crawler-discovered URLs into the ZAP sitemap (sitemap_store).
pub(super) async fn merge_urls_into_sitemap(
    pool: &sqlx::PgPool,
    urls: &[String],
    project_path: Option<&str>,
) {
    if urls.is_empty() { return; }
    let pp = project_path.filter(|s| !s.is_empty());

    let existing: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT data FROM sitemap_store WHERE name = 'zap-sitemap' AND project_path = $1",
    )
    .bind(pp)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let mut sitemap_data = existing.unwrap_or_else(|| serde_json::json!({
        "entries": {},
        "meta": { "source": "katana-merge" },
    }));

    let entries = sitemap_data
        .get_mut("entries")
        .and_then(|e| e.as_object_mut());
    let Some(entries) = entries else {
        tracing::warn!("[katana-sitemap] Could not get entries map from sitemap data");
        return;
    };

    let now = chrono::Utc::now().to_rfc3339();
    let mut added = 0usize;
    for raw_url in urls {
        let parsed = match url::Url::parse(raw_url) {
            Ok(u) => u,
            Err(_) => continue,
        };
        let host = parsed.host_str().unwrap_or("").to_string();
        let path = parsed.path().to_string();
        let dedup_key = format!("GET:{}:{}", host, path);

        if entries.contains_key(&dedup_key) {
            continue;
        }

        let port = parsed.port().unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });

        entries.insert(dedup_key, serde_json::json!({
            "url": raw_url,
            "host": host,
            "method": "GET",
            "path": path,
            "port": port,
            "status_code": 0,
            "content_length": 0,
            "first_seen": &now,
            "last_seen": &now,
            "source": "katana",
            "captured": false,
        }));
        added += 1;
    }

    if added == 0 { return; }

    tracing::info!(
        added = added,
        total = entries.len(),
        "[katana-sitemap] Merged URLs into sitemap"
    );

    let _ = sqlx::query(
        "DELETE FROM sitemap_store WHERE name = 'zap-sitemap' AND project_path = $1",
    )
    .bind(pp)
    .execute(pool)
    .await;

    let _ = sqlx::query(
        r#"INSERT INTO sitemap_store (name, data, project_path)
           VALUES ('zap-sitemap', $1, $2)"#,
    )
    .bind(&sitemap_data)
    .bind(pp)
    .execute(pool)
    .await;
}
