use super::*;
use tempfile::TempDir;

/// Verifies that save_summarizer_input creates a file with the expected content.
#[test]
fn test_save_summarizer_input() {
    let temp_dir = TempDir::new().unwrap();
    let session_id = "test-save";
    let content = "# Test Content\n\nSome conversation here.";

    let path = save_summarizer_input(temp_dir.path(), session_id, content).unwrap();

    assert!(path.exists());
    assert!(path.to_string_lossy().contains("summarizer-input-"));
    assert!(path.to_string_lossy().contains(session_id));

    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(saved, content);
}

/// Verifies that save_summary creates a file with the expected content.
#[test]
fn test_save_summary() {
    let temp_dir = TempDir::new().unwrap();
    let session_id = "test-summary";
    let summary = "## Summary\n\nUser asked for help.";

    let path = save_summary(temp_dir.path(), session_id, summary).unwrap();

    assert!(path.exists());
    assert!(path.to_string_lossy().contains("summary-"));
    assert!(path.to_string_lossy().contains(session_id));

    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(saved, summary);
}

/// Verifies that save_summarizer_input creates nested directories as needed.
#[test]
fn test_save_summarizer_input_creates_directory() {
    let temp_dir = TempDir::new().unwrap();
    let nested_dir = temp_dir.path().join("artifacts").join("compaction");
    let session_id = "test-nested";
    let content = "Test content";

    // Directory doesn't exist yet
    assert!(!nested_dir.exists());

    let path = save_summarizer_input(&nested_dir, session_id, content).unwrap();

    // Directory should now exist
    assert!(nested_dir.exists());
    assert!(path.exists());
}

/// Verifies that save_summary creates nested directories as needed.
#[test]
fn test_save_summary_creates_directory() {
    let temp_dir = TempDir::new().unwrap();
    let nested_dir = temp_dir.path().join("summaries");
    let session_id = "test-dir";
    let summary = "Summary content";

    assert!(!nested_dir.exists());

    let path = save_summary(&nested_dir, session_id, summary).unwrap();

    assert!(nested_dir.exists());
    assert!(path.exists());
}
