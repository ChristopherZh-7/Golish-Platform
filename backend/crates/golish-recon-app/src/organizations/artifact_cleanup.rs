use std::collections::BTreeSet;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use golish_app_core::GolishError;
use golish_cleanup_app::{
    ArtifactCleanupFailure, ArtifactCleanupPlan, OrganizationArtifactCleaner,
};

#[derive(Clone)]
pub struct DbBackedOrganizationArtifactCleaner {
    pool: Arc<PgPool>,
}

impl DbBackedOrganizationArtifactCleaner {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl OrganizationArtifactCleaner for DbBackedOrganizationArtifactCleaner {
    async fn cleanup(&self, plan: &ArtifactCleanupPlan) -> Result<(), ArtifactCleanupFailure> {
        cleanup_frozen_artifacts(&self.pool, plan)
            .await
            .map_err(|error| ArtifactCleanupFailure {
                code: "organization_artifact_cleanup_failed".to_string(),
                message: error.to_string(),
            })
    }
}

#[allow(dead_code)] // retained only for the legacy guard regression test; production uses P7b request().
pub(super) async fn ensure_runtime_scope_deletion_allowed(
    pool: &PgPool,
    org_id: Uuid,
) -> Result<(), GolishError> {
    reject_runtime_scope_history(
        org_id,
        golish_db::repo::operation_org_scope::history_exists_for_org_subtree(pool, org_id).await?,
    )
}

fn reject_runtime_scope_history(org_id: Uuid, history_exists: bool) -> Result<(), GolishError> {
    if history_exists {
        return Err(GolishError::RuntimeScopeHistoryRequiresInvalidation(
            org_id.to_string(),
        ));
    }
    Ok(())
}

#[allow(dead_code)] // legacy one-shot seam; never called by organization_delete after P7b.
pub(super) async fn cleanup_before_delete(pool: &PgPool, org_id: Uuid) -> Result<()> {
    let Some(org) = golish_db::repo::organizations::get_one(pool, org_id).await? else {
        return Ok(());
    };

    let refs =
        golish_db::repo::targets::artifact_reference_values_by_org_subtree(pool, org_id).await?;
    let hosts = hosts_from_values(refs.iter().map(String::as_str));
    let paths_removed = cleanup_host_artifact_dirs(&org.project_path, &hosts).await?;
    let sitemap_entries_removed =
        prune_sitemap_hosts(pool, org.project_path.as_str(), &hosts).await?;
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

/// Execute only from a committed deletion-job snapshot. The DB worker claim is
/// committed before this function receives the plan, so filesystem I/O never
/// overlaps the organization precheck/invalidation transaction.
async fn cleanup_frozen_artifacts(pool: &PgPool, plan: &ArtifactCleanupPlan) -> Result<()> {
    let hosts = hosts_from_values(
        plan.targets
            .iter()
            .map(|target| target.target_value_at_time.as_str()),
    );
    let paths_removed = cleanup_host_artifact_dirs(&plan.project_path_at_time, &hosts).await?;
    let sitemap_entries_removed =
        prune_sitemap_hosts(pool, plan.project_path_at_time.as_str(), &hosts).await?;
    tracing::info!(
        deletion_job_id = %plan.job_id,
        root_organization_id_at_time = %plan.root_organization_id_at_time,
        target_count = plan.targets.len(),
        paths_removed,
        sitemap_entries_removed,
        "organization deletion artifact snapshot cleaned idempotently",
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
    let root = safe_project_root(project_path).await?;
    let mut validated_paths = Vec::new();
    for namespace_name in ["captures", "analysis"] {
        let Some(namespace) = validated_artifact_namespace(&root, namespace_name).await? else {
            continue;
        };
        for host in hosts {
            let candidate = namespace.join(host_slug(host));
            let Some(candidate) = validated_artifact_directory(&namespace, &candidate).await?
            else {
                continue;
            };
            validated_paths.push(candidate);
        }
    }

    let mut removed = 0;
    for path in validated_paths {
        if remove_dir_if_present(&path).await? {
            removed += 1;
        }
    }
    Ok(removed)
}

async fn safe_project_root(raw: &str) -> Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "." {
        bail!("project root is empty or ambiguous");
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        bail!("project root must be an absolute canonical path");
    }
    let canonical = tokio::fs::canonicalize(&path).await.with_context(|| {
        format!(
            "project root is missing or inaccessible: {}",
            path.display()
        )
    })?;
    if canonical != path {
        bail!(
            "project root is not canonical: expected {}, resolved {}",
            path.display(),
            canonical.display()
        );
    }
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .with_context(|| format!("failed to inspect project root {}", canonical.display()))?;
    if !metadata.is_dir() {
        bail!("project root is not a directory: {}", canonical.display());
    }
    Ok(canonical)
}

async fn validated_artifact_namespace(root: &Path, name: &str) -> Result<Option<PathBuf>> {
    let path = root.join(".golish").join(name);
    match tokio::fs::symlink_metadata(&path).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to inspect artifact namespace {}", path.display())
            })
        }
    }
    let canonical = tokio::fs::canonicalize(&path)
        .await
        .with_context(|| format!("failed to resolve artifact namespace {}", path.display()))?;
    if !canonical.starts_with(root) {
        bail!(
            "artifact namespace escapes project root: {} -> {}",
            path.display(),
            canonical.display()
        );
    }
    if !tokio::fs::metadata(&canonical).await?.is_dir() {
        bail!(
            "artifact namespace is not a directory: {}",
            canonical.display()
        );
    }
    Ok(Some(canonical))
}

async fn validated_artifact_directory(
    namespace: &Path,
    candidate: &Path,
) -> Result<Option<PathBuf>> {
    match tokio::fs::symlink_metadata(candidate).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect artifact directory {}",
                    candidate.display()
                )
            })
        }
    }
    let canonical = tokio::fs::canonicalize(candidate).await.with_context(|| {
        format!(
            "failed to resolve artifact directory {}",
            candidate.display()
        )
    })?;
    if !canonical.starts_with(namespace) {
        bail!(
            "artifact directory escapes its project namespace: {} -> {}",
            candidate.display(),
            canonical.display()
        );
    }
    if !tokio::fs::metadata(&canonical).await?.is_dir() {
        bail!("artifact path is not a directory: {}", canonical.display());
    }
    Ok(Some(canonical))
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
    project_path: &str,
    hosts: &BTreeSet<String>,
) -> Result<usize> {
    if hosts.is_empty() {
        return Ok(0);
    }

    for _ in 0..16 {
        let Some(mut data) =
            golish_db::repo::sitemap_store::read_zap_sitemap(pool, Some(project_path)).await?
        else {
            return Ok(0);
        };
        let expected = data.clone();
        let removed = prune_sitemap_data(&mut data, hosts);
        if removed == 0 {
            return Ok(0);
        }
        let has_entries = data
            .get("entries")
            .and_then(Value::as_object)
            .is_some_and(|entries| !entries.is_empty());
        let replacement = has_entries.then_some(&data);
        if golish_db::repo::sitemap_store::compare_and_swap_zap_sitemap(
            pool,
            project_path,
            &expected,
            replacement,
        )
        .await?
        {
            return Ok(removed);
        }
    }
    bail!("sitemap prune concurrent update retry limit exceeded")
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
    fn immutable_runtime_scope_history_returns_the_stable_delete_guard_code() {
        let org_id = Uuid::new_v4();
        let error = reject_runtime_scope_history(org_id, true)
            .expect_err("immutable scope history must block live organization delete");
        assert_eq!(error.code(), "runtime_scope_history_requires_invalidation");
        assert!(reject_runtime_scope_history(org_id, false).is_ok());
    }

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

    #[tokio::test]
    async fn safe_project_root_requires_an_existing_canonical_workspace_path() {
        let project = tempfile::tempdir().expect("temporary project root");
        let canonical = tokio::fs::canonicalize(project.path())
            .await
            .expect("canonical temporary project root");
        assert_eq!(
            safe_project_root(canonical.to_str().expect("utf-8 project root"))
                .await
                .expect("canonical project root"),
            canonical
        );
        assert!(safe_project_root("").await.is_err());
        assert!(safe_project_root(".").await.is_err());
        assert!(safe_project_root("relative/workspace").await.is_err());
    }

    #[tokio::test]
    async fn invalid_project_root_fails_closed_instead_of_reporting_zero_cleanup() {
        let hosts = BTreeSet::from(["example.test".to_string()]);
        let error = cleanup_host_artifact_dirs("relative/workspace", &hosts)
            .await
            .expect_err("an untrusted relative artifact root must be an explicit failure");
        assert!(error.to_string().contains("project root"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn artifact_cleanup_rejects_symlink_escape_without_deleting_external_data() {
        let project = tempfile::tempdir().expect("temporary project root");
        let external = tempfile::tempdir().expect("temporary external root");
        let external_host = external.path().join("example.test");
        tokio::fs::create_dir_all(&external_host)
            .await
            .expect("create external host artifact");
        tokio::fs::create_dir_all(project.path().join(".golish"))
            .await
            .expect("create project metadata directory");
        std::os::unix::fs::symlink(external.path(), project.path().join(".golish/captures"))
            .expect("link capture namespace outside project");

        let hosts = BTreeSet::from(["example.test".to_string()]);
        let canonical_project = tokio::fs::canonicalize(project.path())
            .await
            .expect("canonical project root");
        cleanup_host_artifact_dirs(
            canonical_project.to_str().expect("utf-8 project root"),
            &hosts,
        )
        .await
        .expect_err("a capture namespace symlink escape must fail closed");
        assert!(
            external_host.exists(),
            "cleanup must not follow a project artifact symlink into external data"
        );
    }
}
