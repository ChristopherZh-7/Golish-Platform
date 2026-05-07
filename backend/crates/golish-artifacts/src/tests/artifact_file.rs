use super::*;

#[test]
fn creates_artifact_file() {
    let meta = ArtifactMeta::new(
        PathBuf::from("/path/to/README.md"),
        "Added feature".to_string(),
    );

    let artifact = ArtifactFile::new(
        "README.md".to_string(),
        meta,
        "# Project\n\nDescription here.".to_string(),
    );

    assert_eq!(artifact.filename, "README.md");
    assert!(artifact.content.contains("# Project"));
}

#[test]
fn formats_full_file_content() {
    let meta = ArtifactMeta {
        target: PathBuf::from("/path/to/README.md"),
        created_at: DateTime::parse_from_rfc3339("2025-12-10T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc),
        reason: "Initial creation".to_string(),
        based_on_patches: Vec::new(),
    };

    let artifact = ArtifactFile::new(
        "README.md".to_string(),
        meta,
        "# My Project\n\nWelcome!".to_string(),
    );

    let content = artifact.to_file_content();

    assert!(content.starts_with("<!--"));
    assert!(content.contains("Target: /path/to/README.md"));
    assert!(content.contains("# My Project"));
    assert!(content.contains("Welcome!"));
}

#[test]
fn parses_file_content() {
    let content = r#"<!--
Target: /path/to/CLAUDE.md
Created: 2025-12-10 14:30
Reason: Updated conventions
-->

# CLAUDE.md

Instructions for the AI assistant.

## Commands
- `cargo test` - Run tests"#;

    let artifact = ArtifactFile::from_file_content("CLAUDE.md", content).unwrap();

    assert_eq!(artifact.filename, "CLAUDE.md");
    assert_eq!(artifact.meta.target, PathBuf::from("/path/to/CLAUDE.md"));
    assert!(artifact.content.starts_with("# CLAUDE.md"));
    assert!(artifact.content.contains("## Commands"));
}

#[test]
fn roundtrip_file_content() {
    let meta = ArtifactMeta {
        target: PathBuf::from("/project/README.md"),
        created_at: DateTime::parse_from_rfc3339("2025-12-10T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc),
        reason: "Test roundtrip".to_string(),
        based_on_patches: vec![1, 2],
    };

    let original = ArtifactFile::new(
        "README.md".to_string(),
        meta,
        "# Title\n\nContent here.".to_string(),
    );

    let file_content = original.to_file_content();
    let parsed = ArtifactFile::from_file_content("README.md", &file_content).unwrap();

    assert_eq!(original.filename, parsed.filename);
    assert_eq!(original.meta.target, parsed.meta.target);
    assert_eq!(original.meta.reason, parsed.meta.reason);
    assert_eq!(original.content, parsed.content);
}

#[test]
fn returns_error_for_missing_header() {
    let content = "# Just content, no header";
    let result = ArtifactFile::from_file_content("file.md", content);
    assert!(result.is_err());
}
