//! file_ops integration tests.

use std::fs;
use std::path::Path;

use serde_json::json;
use tempfile::TempDir;

use golish_core::Tool;

use super::*;
use super::*;
use tempfile::tempdir;

// ========================================================================
// read_file tests
// ========================================================================

#[tokio::test]
async fn test_read_file_success() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    fs::write(workspace.join("test.txt"), "hello world").unwrap();

    let tool = ReadFileTool;
    let result = tool
        .execute(json!({"path": "test.txt"}), workspace)
        .await
        .unwrap();

    assert!(result.get("error").is_none());
    assert_eq!(result["content"].as_str().unwrap(), "hello world");
}

#[tokio::test]
async fn test_read_file_not_found() {
    let dir = tempdir().unwrap();
    let tool = ReadFileTool;
    let result = tool
        .execute(json!({"path": "nonexistent.txt"}), dir.path())
        .await
        .unwrap();

    assert!(result.get("error").is_some());
    assert!(result["error"].as_str().unwrap().contains("not found"));
}

#[tokio::test]
async fn test_read_file_line_range() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    fs::write(
        workspace.join("test.txt"),
        "line1\nline2\nline3\nline4\nline5",
    )
    .unwrap();

    let tool = ReadFileTool;
    let result = tool
        .execute(
            json!({"path": "test.txt", "line_start": 2, "line_end": 4}),
            workspace,
        )
        .await
        .unwrap();

    assert!(result.get("error").is_none());
    assert_eq!(result["content"].as_str().unwrap(), "line2\nline3\nline4");
}

#[tokio::test]
async fn test_read_file_binary_detection() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Write binary content (contains null bytes)
    fs::write(workspace.join("binary.bin"), b"hello\x00world").unwrap();

    let tool = ReadFileTool;
    let result = tool
        .execute(json!({"path": "binary.bin"}), workspace)
        .await
        .unwrap();

    assert!(result.get("error").is_some());
    assert!(result["error"].as_str().unwrap().contains("binary"));
}

#[tokio::test]
async fn test_read_file_missing_path_arg() {
    let dir = tempdir().unwrap();
    let tool = ReadFileTool;
    let result = tool.execute(json!({}), dir.path()).await.unwrap();

    assert!(result.get("error").is_some());
    assert!(result["error"].as_str().unwrap().contains("Missing"));
}

// ========================================================================
// write_file tests
// ========================================================================

#[tokio::test]
async fn test_write_file_creates_new() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    let tool = WriteFileTool;
    let result = tool
        .execute(
            json!({"path": "new.txt", "content": "new content"}),
            workspace,
        )
        .await
        .unwrap();

    assert!(result.get("error").is_none());
    assert_eq!(result["success"].as_bool(), Some(true));
    assert_eq!(
        fs::read_to_string(workspace.join("new.txt")).unwrap(),
        "new content"
    );
}

#[tokio::test]
async fn test_write_file_overwrites() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    fs::write(workspace.join("existing.txt"), "old content").unwrap();

    let tool = WriteFileTool;
    let result = tool
        .execute(
            json!({"path": "existing.txt", "content": "new content"}),
            workspace,
        )
        .await
        .unwrap();

    assert!(result.get("error").is_none());
    assert_eq!(
        fs::read_to_string(workspace.join("existing.txt")).unwrap(),
        "new content"
    );
}

#[tokio::test]
async fn test_write_file_creates_parent_dirs() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    let tool = WriteFileTool;
    let result = tool
        .execute(
            json!({"path": "deep/nested/dir/file.txt", "content": "content"}),
            workspace,
        )
        .await
        .unwrap();

    assert!(result.get("error").is_none());
    assert!(workspace.join("deep/nested/dir/file.txt").exists());
}

// ========================================================================
// create_file tests
// ========================================================================

#[tokio::test]
async fn test_create_file_new() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    let tool = CreateFileTool;
    let result = tool
        .execute(json!({"path": "new.txt", "content": "content"}), workspace)
        .await
        .unwrap();

    assert!(result.get("error").is_none());
    assert_eq!(result["success"].as_bool(), Some(true));
}

#[tokio::test]
async fn test_create_file_fails_if_exists() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    fs::write(workspace.join("existing.txt"), "existing").unwrap();

    let tool = CreateFileTool;
    let result = tool
        .execute(json!({"path": "existing.txt", "content": "new"}), workspace)
        .await
        .unwrap();

    assert!(result.get("error").is_some());
    assert!(result["error"].as_str().unwrap().contains("exists"));
}

// ========================================================================
// edit_file tests
// ========================================================================

#[tokio::test]
async fn test_edit_file_success() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    fs::write(workspace.join("test.txt"), "hello world").unwrap();

    let tool = EditFileTool;
    let result = tool
        .execute(
            json!({
                "path": "test.txt",
                "old_text": "hello",
                "new_text": "goodbye"
            }),
            workspace,
        )
        .await
        .unwrap();

    assert!(result.get("error").is_none());
    assert_eq!(result["success"].as_bool(), Some(true));
    assert_eq!(
        fs::read_to_string(workspace.join("test.txt")).unwrap(),
        "goodbye world"
    );
}

#[tokio::test]
async fn test_edit_file_no_match() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    fs::write(workspace.join("test.txt"), "hello world").unwrap();

    let tool = EditFileTool;
    let result = tool
        .execute(
            json!({
                "path": "test.txt",
                "old_text": "nonexistent",
                "new_text": "replacement"
            }),
            workspace,
        )
        .await
        .unwrap();

    assert!(result.get("error").is_some());
    assert!(result["error"].as_str().unwrap().contains("no matches"));
}

#[tokio::test]
async fn test_edit_file_multiple_matches() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    fs::write(workspace.join("test.txt"), "hello hello hello").unwrap();

    let tool = EditFileTool;
    let result = tool
        .execute(
            json!({
                "path": "test.txt",
                "old_text": "hello",
                "new_text": "goodbye"
            }),
            workspace,
        )
        .await
        .unwrap();

    assert!(result.get("error").is_some());
    assert!(result["error"].as_str().unwrap().contains("3 matches"));
}

#[tokio::test]
async fn test_edit_file_returns_diff() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    fs::write(workspace.join("test.txt"), "line1\nline2\nline3").unwrap();

    let tool = EditFileTool;
    let result = tool
        .execute(
            json!({
                "path": "test.txt",
                "old_text": "line2",
                "new_text": "modified"
            }),
            workspace,
        )
        .await
        .unwrap();

    assert!(result.get("error").is_none());
    assert!(result.get("diff").is_some());
    let diff = result["diff"].as_str().unwrap();
    assert!(diff.contains("-line2"));
    assert!(diff.contains("+modified"));
}

// ========================================================================
// delete_file tests
// ========================================================================

#[tokio::test]
async fn test_delete_file_success() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    fs::write(workspace.join("to_delete.txt"), "content").unwrap();
    assert!(workspace.join("to_delete.txt").exists());

    let tool = DeleteFileTool;
    let result = tool
        .execute(json!({"path": "to_delete.txt"}), workspace)
        .await
        .unwrap();

    assert!(result.get("error").is_none());
    assert_eq!(result["success"].as_bool(), Some(true));
    assert!(!workspace.join("to_delete.txt").exists());
}

#[tokio::test]
async fn test_delete_file_not_found() {
    let dir = tempdir().unwrap();

    let tool = DeleteFileTool;
    let result = tool
        .execute(json!({"path": "nonexistent.txt"}), dir.path())
        .await
        .unwrap();

    assert!(result.get("error").is_some());
    assert!(result["error"].as_str().unwrap().contains("not found"));
}

#[tokio::test]
async fn test_delete_file_is_directory() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    fs::create_dir(workspace.join("subdir")).unwrap();

    let tool = DeleteFileTool;
    let result = tool
        .execute(json!({"path": "subdir"}), workspace)
        .await
        .unwrap();

    assert!(result.get("error").is_some());
    assert!(result["error"].as_str().unwrap().contains("directory"));
}

// ========================================================================
// Path security tests
// ========================================================================

#[tokio::test]
async fn test_path_traversal_blocked() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    // Create a file outside the workspace
    let parent = workspace.parent().unwrap();
    fs::write(parent.join("outside.txt"), "secret").unwrap();

    let tool = ReadFileTool;
    let result = tool
        .execute(json!({"path": "../outside.txt"}), workspace)
        .await
        .unwrap();

    assert!(result.get("error").is_some());
    assert!(result["error"]
        .as_str()
        .unwrap()
        .contains("outside workspace"));
}
