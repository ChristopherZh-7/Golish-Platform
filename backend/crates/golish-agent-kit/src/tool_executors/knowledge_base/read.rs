//! `read_knowledge` — read one exact logical wiki page.

use std::path::{Component, Path, PathBuf};

use serde_json::json;

use crate::tool_executors::common::{error_result, extract_string_param, ToolResult};

use super::wiki::wiki_base_dir;

pub(super) async fn handle_read(
    args: &serde_json::Value,
    db_tracker: Option<&crate::db_tracking::DbTracker>,
) -> ToolResult {
    let path = match extract_string_param(args, &["path"]) {
        Some(p) if !p.is_empty() => p,
        _ => return error_result("read_knowledge requires a 'path' parameter"),
    };
    let relative_path = match safe_relative_wiki_path(&path) {
        Some(path) => path,
        None => return error_result("read_knowledge requires a safe relative wiki path"),
    };

    // Search is DB-backed whenever a tracker is installed. Read the exact same
    // logical row first so an indexed page stays readable even when the runtime
    // resource root differs from the process that populated the index.
    let indexed_page = if let Some(repo) = db_tracker.and_then(crate::db_tracking::DbTracker::repo)
    {
        match crate::db_shim::wiki_kb::get_page(repo, &path).await {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(
                    path,
                    %error,
                    "[kb] exact DB page read failed; trying the local wiki source"
                );
                None
            }
        }
    } else {
        None
    };

    exact_or_local_page_result(
        &path,
        &relative_path,
        indexed_page.as_ref(),
        &wiki_base_dir(),
    )
    .await
}

async fn exact_or_local_page_result(
    path: &str,
    relative_path: &Path,
    indexed_page: Option<&serde_json::Value>,
    local_wiki_root: &Path,
) -> ToolResult {
    if let Some(page) = indexed_page {
        return database_page_result(path, page);
    }

    let full = local_wiki_root.join(relative_path);
    match tokio::fs::read_to_string(&full).await {
        Ok(content) => (
            json!({
                "path": path,
                "content": content,
            }),
            true,
        ),
        Err(e) => error_result(format!("File not found or unreadable: {} ({})", path, e)),
    }
}

fn safe_relative_wiki_path(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(path.to_path_buf())
}

fn database_page_result(path: &str, page: &serde_json::Value) -> ToolResult {
    let Some(content) = page.get("content").and_then(serde_json::Value::as_str) else {
        return error_result(format!(
            "Indexed knowledge page has no readable content: {path}"
        ));
    };
    let Some(indexed_path) = page.get("path").and_then(serde_json::Value::as_str) else {
        return error_result(format!(
            "Indexed knowledge page has no canonical path: {path}"
        ));
    };
    if indexed_path != path {
        return error_result(format!(
            "Indexed knowledge page path mismatch: requested {path}, received {indexed_path}"
        ));
    }
    (
        json!({
            "path": indexed_path,
            "content": content,
            "source": "indexed_wiki_page",
        }),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn indexed_unicode_page_uses_the_exact_search_path_without_a_local_file() {
        let path = "analysis/默安科技/target-overview.md";
        let page = json!({
            "path": path,
            "content": "# 默安科技\n\nPrimary: moresec.cn",
        });
        let absent_local_root = std::env::temp_dir().join(format!(
            "golish-indexed-kb-page-must-not-read-local-{}",
            uuid::Uuid::new_v4()
        ));

        let (result, success) =
            exact_or_local_page_result(path, Path::new(path), Some(&page), &absent_local_root)
                .await;

        assert!(success);
        assert_eq!(result["path"], path);
        assert_eq!(result["content"], "# 默安科技\n\nPrimary: moresec.cn");
        assert_eq!(result["source"], "indexed_wiki_page");
    }

    #[test]
    fn exact_page_read_rejects_path_escape_and_db_path_drift() {
        assert!(safe_relative_wiki_path("analysis/默安科技/target-overview.md").is_some());
        assert!(safe_relative_wiki_path("../secrets.txt").is_none());
        assert!(safe_relative_wiki_path("/etc/passwd").is_none());

        let (_, success) = database_page_result(
            "analysis/默安科技/target-overview.md",
            &json!({
                "path": "analysis/another/page.md",
                "content": "wrong page",
            }),
        );
        assert!(!success);
    }
}
