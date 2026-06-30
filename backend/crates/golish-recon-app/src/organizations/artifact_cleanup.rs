use std::collections::BTreeSet;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

pub(super) async fn cleanup_before_delete(pool: &PgPool, org_id: Uuid) -> Result<()> {
    let Some(org) = golish_db::repo::organizations::get_one(pool, org_id).await? else {
        return Ok(());
    };

    let refs =
        golish_db::repo::targets::artifact_reference_values_by_org_subtree(pool, org_id).await?;
    let hosts = hosts_from_values(refs.iter().map(String::as_str));
    let paths_removed = cleanup_host_artifact_dirs(&org.project_path, &hosts).await?;
    let sitemap_entries_removed =
        prune_sitemap_hosts(pool, Some(org.project_path.as_str()), &hosts).await?;
    let operation_bindings_cleared =
        golish_db::repo::operation_state::clear_engagement_org_for_subtree(pool, org_id).await?;

    tracing::info!(
        organization_id = %org_id,
        host_count = hosts.len(),
        paths_removed,
        sitemap_entries_removed,
        operation_bindings_cleared,
        "organization delete cleaned target artifacts",
    );

    Ok(())
}

fn hosts_from_values<'a>(values: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    values.into_iter().filter_map(host_from_reference).collect()
}

fn host_from_reference(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_end_matches(',');
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return None;
    }

    if let Some(host) = parse_url_host(trimmed) {
        return Some(host);
    }

    let wildcard_url = trimmed.replace("://*.", "://");
    if wildcard_url != trimmed {
        if let Some(host) = parse_url_host(&wildcard_url) {
            return Some(host);
        }
    }

    let first_token = trimmed.split_whitespace().next().unwrap_or(trimmed);
    let no_wildcard = first_token.strip_prefix("*.").unwrap_or(first_token);
    let candidate = no_wildcard.split('/').next().unwrap_or(no_wildcard);
    if candidate.is_empty() {
        return None;
    }

    parse_url_host(&format!("http://{candidate}")).or_else(|| normalize_host(candidate))
}

fn parse_url_host(raw: &str) -> Option<String> {
    Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().and_then(normalize_host))
}

fn normalize_host(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches("*.").trim_end_matches('.');
    if trimmed.is_empty() {
        return None;
    }

    let unbracketed = if let Some(rest) = trimmed.strip_prefix('[') {
        rest.find(']').map(|end| &rest[..end]).unwrap_or(trimmed)
    } else {
        trimmed
    };
    if unbracketed.parse::<IpAddr>().is_ok() {
        return Some(unbracketed.to_ascii_lowercase());
    }

    let candidate = Url::parse(&format!("http://{trimmed}"))
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| unbracketed.to_string())
        .trim_end_matches('.')
        .to_ascii_lowercase();

    if candidate == "localhost" || candidate.contains('.') || candidate.parse::<IpAddr>().is_ok() {
        Some(candidate)
    } else {
        None
    }
}

async fn cleanup_host_artifact_dirs(project_path: &str, hosts: &BTreeSet<String>) -> Result<usize> {
    let Some(root) = safe_project_root(project_path) else {
        return Ok(0);
    };

    let mut removed = 0;
    for host in hosts {
        let slug = host_slug(host);
        let paths = [
            root.join(".golish").join("captures").join(&slug),
            root.join(".golish").join("analysis").join(&slug),
        ];
        for path in paths {
            if remove_dir_if_present(&path).await? {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

fn safe_project_root(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "." {
        return None;
    }
    let path = PathBuf::from(trimmed);
    path.is_absolute().then_some(path)
}

async fn remove_dir_if_present(path: &Path) -> Result<bool> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("failed to remove {}", path.display())),
    }
}

async fn prune_sitemap_hosts(
    pool: &PgPool,
    project_path: Option<&str>,
    hosts: &BTreeSet<String>,
) -> Result<usize> {
    if hosts.is_empty() {
        return Ok(0);
    }

    let Some(mut data) =
        golish_db::repo::sitemap_store::read_zap_sitemap(pool, project_path).await?
    else {
        return Ok(0);
    };

    let removed = prune_sitemap_data(&mut data, hosts);
    if removed == 0 {
        return Ok(0);
    }

    golish_db::repo::sitemap_store::delete_zap_sitemap(pool, project_path).await?;
    let has_entries = data
        .get("entries")
        .and_then(Value::as_object)
        .is_some_and(|entries| !entries.is_empty());
    if has_entries {
        golish_db::repo::sitemap_store::upsert_zap_sitemap(pool, project_path, &data).await?;
    }
    Ok(removed)
}

fn prune_sitemap_data(data: &mut Value, hosts: &BTreeSet<String>) -> usize {
    let Some(entries) = data.get_mut("entries").and_then(Value::as_object_mut) else {
        return 0;
    };
    let before = entries.len();
    entries.retain(|_, entry| !sitemap_entry_matches_hosts(entry, hosts));
    before.saturating_sub(entries.len())
}

fn sitemap_entry_matches_hosts(entry: &Value, hosts: &BTreeSet<String>) -> bool {
    entry
        .get("host")
        .and_then(Value::as_str)
        .and_then(normalize_host)
        .is_some_and(|host| hosts.contains(&host))
        || entry
            .get("url")
            .and_then(Value::as_str)
            .and_then(host_from_reference)
            .is_some_and(|host| hosts.contains(&host))
        || entry
            .pointer("/capture/local_path")
            .and_then(Value::as_str)
            .is_some_and(|path| capture_path_matches_hosts(path, hosts))
}

fn capture_path_matches_hosts(path: &str, hosts: &BTreeSet<String>) -> bool {
    let normalized = path.replace('\\', "/");
    hosts.iter().any(|host| {
        let slug = host_slug(host);
        normalized.contains(&format!(".golish/captures/{slug}/"))
            || normalized.starts_with(&format!(".golish/captures/{slug}/"))
    })
}

fn host_slug(host: &str) -> String {
    host.replace(['/', '\\'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn host_extraction_handles_urls_wildcards_ips_and_noise() {
        let hosts = hosts_from_values([
            "https://Example.com:8443/assets/app.js",
            "example.org/path",
            "*.api.example.net",
            "http://*.wild.example.net/main.js",
            "10.0.0.1:8080",
            "[::1]:9443",
            "AS12345",
            "/relative/api",
        ]);

        assert!(hosts.contains("example.com"));
        assert!(hosts.contains("example.org"));
        assert!(hosts.contains("api.example.net"));
        assert!(hosts.contains("wild.example.net"));
        assert!(hosts.contains("10.0.0.1"));
        assert!(hosts.contains("::1"));
        assert!(!hosts.contains("as12345"));
    }

    #[test]
    fn sitemap_pruning_removes_matching_hosts_urls_and_capture_paths() {
        let hosts = BTreeSet::from(["example.com".to_string(), "api.example.net".to_string()]);
        let mut data = json!({
            "entries": {
                "GET:https://example.com/app.js": {
                    "url": "https://example.com/app.js",
                    "host": "example.com"
                },
                "GET:https://api.example.net/app.js": {
                    "url": "https://api.example.net/app.js",
                    "capture": {
                        "local_path": ".golish/captures/api.example.net/443/js/app.js"
                    }
                },
                "GET:https://other.example/app.js": {
                    "url": "https://other.example/app.js",
                    "host": "other.example"
                }
            }
        });

        assert_eq!(prune_sitemap_data(&mut data, &hosts), 2);
        let entries = data["entries"].as_object().expect("entries object");
        assert_eq!(entries.len(), 1);
        assert!(entries.contains_key("GET:https://other.example/app.js"));
    }

    #[test]
    fn safe_project_root_requires_absolute_workspace_path() {
        assert!(safe_project_root("/tmp/workspace").is_some());
        assert!(safe_project_root("").is_none());
        assert!(safe_project_root(".").is_none());
        assert!(safe_project_root("relative/workspace").is_none());
    }
}
