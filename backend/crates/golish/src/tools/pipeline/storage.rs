//! Adapter implementing [`golish_pipeline::PipelineStorage`] in terms of
//! the main crate's existing DB helpers. The business logic lives in
//! `golish-pipeline`; this module is purely about delegation.

use async_trait::async_trait;
use golish_pipeline::{extract_hostname, ParsedItem, PipelineError, PipelineResult, PipelineStorage};
use sqlx::PgPool;
use uuid::Uuid;

use crate::tools::targets;

/// Main-crate adapter: maps the pipeline engine's storage callbacks to
/// `crate::tools::targets::*` and `crate::tools::output_parser::*`.
pub struct MainStorage;

#[async_trait]
impl PipelineStorage for MainStorage {
    async fn store_target_from_item(
        &self,
        pool: &PgPool,
        item: &ParsedItem,
        project_path: Option<&str>,
        parent_id: Option<Uuid>,
    ) -> PipelineResult<bool> {
        let hostname = if let Some(h) = item
            .fields
            .get("hostname")
            .or_else(|| item.fields.get("host"))
            .or_else(|| item.fields.get("ip"))
        {
            h.clone()
        } else if let Some(url) = item.fields.get("url") {
            extract_hostname(url)
        } else {
            return Err(PipelineError::Storage("No hostname/host/ip/url field".into()));
        };

        let existed = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM targets WHERE value = $1 AND project_path = $2)",
        )
        .bind(&hostname)
        .bind(project_path)
        .fetch_one(pool)
        .await
        .unwrap_or(false);

        targets::db_target_add(
            pool,
            &hostname,
            &hostname,
            None,
            project_path,
            "discovered",
            parent_id,
        )
        .await
        .map_err(PipelineError::Storage)?;
        Ok(!existed)
    }

    async fn store_recon_from_item(
        &self,
        pool: &PgPool,
        item: &ParsedItem,
        project_path: Option<&str>,
    ) -> PipelineResult<bool> {
        let host_val = item
            .fields
            .get("host")
            .or_else(|| item.fields.get("ip"))
            .or_else(|| item.fields.get("url"))
            .ok_or_else(|| PipelineError::Storage("No host/ip field".into()))?;

        let hostname = extract_hostname(host_val);
        let target =         targets::db_target_add(
            pool,
            &hostname,
            &hostname,
            None,
            project_path,
            "discovered",
            None,
        )
        .await
        .map_err(PipelineError::Storage)?;
        let target_uuid: Uuid = target.id.parse().map_err(|e: uuid::Error| PipelineError::Storage(e.to_string()))?;

        let mut update = targets::ReconUpdate::new();
        if let Some(ip) = item.fields.get("ip") {
            update.real_ip = ip.clone();
        }
        if let Some(cdn) = item.fields.get("cdn") {
            update.cdn_waf = cdn.clone();
        }
        if let Some(os) = item.fields.get("os") {
            update.os_info = os.clone();
        }

        if let Some(port_entry) = golish_pentest::output_store::build_port_json(&item.fields) {
            update.ports = serde_json::json!([port_entry]);
        }

        let is_new_port = if let Some(port_str) = item.fields.get("port") {
            let port_num: i32 = port_str.parse().unwrap_or(0);
            let proto = item
                .fields
                .get("protocol")
                .cloned()
                .unwrap_or_else(|| "tcp".to_string());
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
        if let Some(status) = item
            .fields
            .get("status_code")
            .or_else(|| item.fields.get("status"))
        {
            update.http_status = status.parse().ok();
        }
        if let Some(ws) = item.fields.get("webserver") {
            update.webserver = ws.clone();
        }

        targets::db_target_update_recon_extended(pool, target_uuid, &update).await.map_err(PipelineError::Storage)?;

        let tool_source = item
            .fields
            .get("_tool")
            .map(|s| s.as_str())
            .unwrap_or("httpx");
        golish_pentest::output_store::store_fingerprints(
            pool,
            target_uuid,
            project_path,
            &item.fields,
            tool_source,
        )
        .await;

        Ok(is_new_port)
    }

    async fn store_dirent_from_item(
        &self,
        pool: &PgPool,
        item: &ParsedItem,
        tool_name: &str,
        project_path: Option<&str>,
    ) -> PipelineResult<bool> {
        let url = item.fields.get("url").ok_or_else(|| PipelineError::Storage("No url field".into()))?;

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

        targets::db_directory_entry_add(
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
        .await
        .map_err(PipelineError::Storage)?;
        Ok(!existed)
    }

    async fn store_finding_from_item(
        &self,
        pool: &PgPool,
        item: &ParsedItem,
        tool_name: &str,
        project_path: Option<&str>,
    ) -> PipelineResult<bool> {
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
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn merge_urls_into_sitemap(
        &self,
        pool: &PgPool,
        urls: &[String],
        project_path: Option<&str>,
    ) {
        if urls.is_empty() {
            return;
        }
        let pp = project_path.filter(|s| !s.is_empty());

        let existing: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT data FROM sitemap_store WHERE name = 'zap-sitemap' AND project_path = $1",
        )
        .bind(pp)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

        let mut sitemap_data = existing.unwrap_or_else(|| {
            serde_json::json!({
                "entries": {},
                "meta": { "source": "katana-merge" },
            })
        });

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

            let port = parsed
                .port()
                .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });

            entries.insert(
                dedup_key,
                serde_json::json!({
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
                }),
            );
            added += 1;
        }

        if added == 0 {
            return;
        }

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
}
