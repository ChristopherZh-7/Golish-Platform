//! Fingerprint → PoC matching engine.
//!
//! Looks up cached PoCs/Nuclei templates relevant to the fingerprints
//! recorded for a target and returns ranked `PocMatch`es.

use uuid::Uuid;

use super::severity_rank;
use crate::types::PocMatch;

pub async fn match_pocs_for_target(
    pool: &sqlx::PgPool,
    target_id: Uuid,
) -> crate::ScanRunnerResult<Vec<PocMatch>> {
    let start = std::time::Instant::now();
    let mut fingerprints = golish_db::repo::fingerprints::list_by_target(pool, target_id).await?;

    if fingerprints.is_empty() {
        let backfilled = backfill_fingerprints_from_target(pool, target_id).await;
        if backfilled > 0 {
            tracing::info!(
                "[PoC-Match] Backfilled {} fingerprints from targets table for {}",
                backfilled,
                target_id
            );
            fingerprints = golish_db::repo::fingerprints::list_by_target(pool, target_id).await?;
        }
    }

    if fingerprints.is_empty() {
        tracing::info!(
            "[PoC-Match] 0 fingerprints for target {} after backfill attempt ({}ms)",
            target_id,
            start.elapsed().as_millis()
        );
        return Ok(vec![]);
    }

    tracing::info!(
        "[PoC-Match] {} fingerprints for target {} ({}ms): {:?}",
        fingerprints.len(),
        target_id,
        start.elapsed().as_millis(),
        fingerprints
            .iter()
            .map(|f| format!("{}:{}", f.category, f.name))
            .collect::<Vec<_>>()
    );

    let mut all_terms: Vec<(String, String, String)> = Vec::new();
    let mut tag_terms: Vec<String> = Vec::new();
    for fp in &fingerprints {
        let name_lower = fp.name.to_lowercase();
        let version = fp.version.clone().unwrap_or_default();
        tag_terms.push(name_lower.clone());
        for term in build_search_terms(&fp.name, fp.version.as_deref()) {
            all_terms.push((term, name_lower.clone(), version.clone()));
        }
    }

    let combined_pattern = all_terms
        .iter()
        .map(|(t, _, _)| regex::escape(t))
        .collect::<Vec<_>>()
        .join("|");

    let q_start = std::time::Instant::now();

    let rows_text = sqlx::query_as::<_, PocRow>(
        r#"SELECT DISTINCT id, cve_id, name, poc_type, severity, source, content
           FROM vuln_kb_pocs
           WHERE LOWER(name) ~* $1
              OR LOWER(cve_id) ~* $1
              OR LOWER(description) ~* $1
           LIMIT 200"#,
    )
    .bind(&combined_pattern)
    .fetch_all(pool)
    .await?;

    let rows_tags = sqlx::query_as::<_, PocRow>(
        r#"SELECT DISTINCT id, cve_id, name, poc_type, severity, source, content
           FROM vuln_kb_pocs
           WHERE tags && $1
           LIMIT 200"#,
    )
    .bind(&tag_terms)
    .fetch_all(pool)
    .await?;

    let mut rows = rows_text;
    rows.extend(rows_tags);
    tracing::info!(
        "[PoC-Match] Queries returned {} rows ({}ms)",
        rows.len(),
        q_start.elapsed().as_millis()
    );

    let mut seen_ids = std::collections::HashSet::new();
    let mut matches = Vec::new();

    for row in rows {
        let row_id_str = row.id.to_string();
        if !seen_ids.insert(row_id_str.clone()) {
            continue;
        }

        let row_name_lower = row.name.to_lowercase();
        let row_cve_lower = row.cve_id.to_lowercase();

        let matched = all_terms
            .iter()
            .find(|(term, _, _)| row_name_lower.contains(term) || row_cve_lower.contains(term));

        let (fp_name, fp_ver) = match matched {
            Some((_, n, v)) => (n.clone(), v.clone()),
            None => all_terms
                .first()
                .map(|(_, n, v)| (n.clone(), v.clone()))
                .unwrap_or_default(),
        };

        let template_id = extract_nuclei_template_id(&row.content);

        matches.push(PocMatch {
            poc_id: row_id_str,
            cve_id: row.cve_id,
            poc_name: row.name,
            poc_type: row.poc_type,
            severity: row.severity.unwrap_or_default(),
            source: row.source.unwrap_or_default(),
            matched_fingerprint: fp_name,
            matched_version: fp_ver,
            template_id,
        });
    }

    matches.sort_by_key(|m| std::cmp::Reverse(severity_rank(&m.severity)));

    tracing::info!(
        "[PoC-Match] Total {} matches in {}ms",
        matches.len(),
        start.elapsed().as_millis()
    );
    Ok(matches)
}

#[derive(sqlx::FromRow)]
struct PocRow {
    id: Uuid,
    cve_id: String,
    name: String,
    poc_type: String,
    severity: Option<String>,
    source: Option<String>,
    content: String,
}

async fn backfill_fingerprints_from_target(pool: &sqlx::PgPool, target_id: Uuid) -> u32 {
    let row: Option<(String, String, String, sqlx::types::Json<serde_json::Value>, String)> =
        sqlx::query_as(
            "SELECT webserver, cdn_waf, os_info, ports, COALESCE(project_path, '') FROM targets WHERE id = $1",
        )
        .bind(target_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

    let Some((ws, cdn, os, ports, project_path)) = row else {
        return 0;
    };
    let pp = if project_path.is_empty() {
        None
    } else {
        Some(project_path.as_str())
    };
    let mut count = 0u32;

    fn parse_sv(s: &str) -> (String, Option<String>) {
        let s = s.trim();
        if let Some(idx) = s.find('/') {
            let name = s[..idx].trim().to_string();
            let ver = s[idx + 1..].trim().to_string();
            if ver.is_empty() {
                (name, None)
            } else {
                (name, Some(ver))
            }
        } else {
            (s.to_string(), None)
        }
    }

    if !ws.is_empty() {
        let (name, version) = parse_sv(&ws);
        let ev = serde_json::json!({ "source": "backfill", "raw": ws });
        if golish_db::repo::fingerprints::upsert(
            pool,
            target_id,
            pp,
            "webserver",
            &name,
            version.as_deref(),
            0.8,
            &ev,
            None,
            "httpx",
        )
        .await
        .is_ok()
        {
            count += 1;
        }
    }
    if !cdn.is_empty() {
        let ev = serde_json::json!({ "source": "backfill", "raw": cdn });
        if golish_db::repo::fingerprints::upsert(
            pool, target_id, pp, "cdn", &cdn, None, 0.9, &ev, None, "httpx",
        )
        .await
        .is_ok()
        {
            count += 1;
        }
    }
    if !os.is_empty() {
        let (name, version) = parse_sv(&os);
        let ev = serde_json::json!({ "source": "backfill", "raw": os });
        if golish_db::repo::fingerprints::upsert(
            pool,
            target_id,
            pp,
            "os",
            &name,
            version.as_deref(),
            0.6,
            &ev,
            None,
            "httpx",
        )
        .await
        .is_ok()
        {
            count += 1;
        }
    }

    if let Some(arr) = ports.0.as_array() {
        for port_entry in arr {
            if let Some(techs) = port_entry.get("technologies").and_then(|t| t.as_array()) {
                for tech_val in techs {
                    if let Some(tech) = tech_val.as_str() {
                        if !tech.is_empty() {
                            let (name, version) = parse_sv(tech);
                            let ev = serde_json::json!({ "source": "backfill", "port": port_entry.get("port") });
                            if golish_db::repo::fingerprints::upsert(
                                pool,
                                target_id,
                                pp,
                                "technology",
                                &name,
                                version.as_deref(),
                                0.7,
                                &ev,
                                None,
                                "httpx",
                            )
                            .await
                            .is_ok()
                            {
                                count += 1;
                            }
                        }
                    }
                }
            }
            if let Some(ws_val) = port_entry.get("webserver").and_then(|w| w.as_str()) {
                if !ws_val.is_empty() {
                    let (name, version) = parse_sv(ws_val);
                    let ev =
                        serde_json::json!({ "source": "backfill", "port": port_entry.get("port") });
                    if golish_db::repo::fingerprints::upsert(
                        pool,
                        target_id,
                        pp,
                        "webserver",
                        &name,
                        version.as_deref(),
                        0.8,
                        &ev,
                        None,
                        "httpx",
                    )
                    .await
                    .is_ok()
                    {
                        count += 1;
                    }
                }
            }
        }
    }

    count
}

fn build_search_terms(name: &str, version: Option<&str>) -> Vec<String> {
    let lower = name.to_lowercase();
    let mut terms = vec![lower.clone()];

    let mapped = match lower.as_str() {
        "apache" => Some("apache"),
        "nginx" => Some("nginx"),
        "iis" | "microsoft-iis" => Some("iis"),
        "tomcat" | "apache-tomcat" => Some("tomcat"),
        "wordpress" => Some("wordpress"),
        "drupal" => Some("drupal"),
        "joomla" => Some("joomla"),
        "php" => Some("php"),
        "jquery" => Some("jquery"),
        "spring" | "spring-boot" | "spring-framework" => Some("spring"),
        "struts" | "apache-struts" => Some("struts"),
        "log4j" => Some("log4j"),
        "openssl" => Some("openssl"),
        "jenkins" => Some("jenkins"),
        "gitlab" => Some("gitlab"),
        "grafana" => Some("grafana"),
        "elasticsearch" => Some("elasticsearch"),
        "redis" => Some("redis"),
        "mongodb" | "mongo" => Some("mongodb"),
        _ => None,
    };
    if let Some(m) = mapped {
        if m != lower {
            terms.push(m.to_string());
        }
    }

    if let Some(ver) = version {
        terms.push(format!("{} {}", lower, ver));
    }

    terms
}

fn extract_nuclei_template_id(content: &str) -> Option<String> {
    for line in content.lines().take(20) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("id:") {
            return Some(rest.trim().to_string());
        }
    }
    None
}
