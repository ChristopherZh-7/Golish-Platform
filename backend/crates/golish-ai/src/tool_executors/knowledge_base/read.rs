//! `read_knowledge` — read a single wiki page from disk.

use serde_json::json;

use crate::tool_executors::common::{error_result, extract_string_param, ToolResult};

use super::wiki::wiki_base_dir;

pub(super) async fn handle_read(args: &serde_json::Value) -> ToolResult {
    let path = match extract_string_param(args, &["path"]) {
        Some(p) if !p.is_empty() => p,
        _ => return error_result("read_knowledge requires a 'path' parameter"),
    };

    let full = wiki_base_dir().join(&path);
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
