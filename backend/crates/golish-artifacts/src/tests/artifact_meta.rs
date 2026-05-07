use super::*;

#[test]
fn creates_new_metadata() {
    let meta = ArtifactMeta::new(
        PathBuf::from("/path/to/README.md"),
        "Added authentication".to_string(),
    );

    assert_eq!(meta.target, PathBuf::from("/path/to/README.md"));
    assert_eq!(meta.reason, "Added authentication");
    assert!(meta.based_on_patches.is_empty());
}

#[test]
fn creates_metadata_with_patches() {
    let meta = ArtifactMeta::with_patches(
        PathBuf::from("/path/to/README.md"),
        "Added auth".to_string(),
        vec![1, 2, 3],
    );

    assert_eq!(meta.based_on_patches, vec![1, 2, 3]);
}

#[test]
fn formats_header_without_patches() {
    let meta = ArtifactMeta {
        target: PathBuf::from("/path/to/README.md"),
        created_at: DateTime::parse_from_rfc3339("2025-12-10T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc),
        reason: "Added authentication feature".to_string(),
        based_on_patches: Vec::new(),
    };

    let header = meta.to_header();

    assert!(header.starts_with("<!--"));
    assert!(header.ends_with("-->"));
    assert!(header.contains("Target: /path/to/README.md"));
    assert!(header.contains("Created: 2025-12-10 14:30"));
    assert!(header.contains("Reason: Added authentication feature"));
    assert!(!header.contains("Based on patches"));
}

#[test]
fn formats_header_with_patches() {
    let meta = ArtifactMeta {
        target: PathBuf::from("/path/to/README.md"),
        created_at: DateTime::parse_from_rfc3339("2025-12-10T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc),
        reason: "Added authentication".to_string(),
        based_on_patches: vec![1, 2],
    };

    let header = meta.to_header();

    assert!(header.contains("Based on patches: 0001, 0002"));
}

#[test]
fn parses_header_without_patches() {
    let header = r#"<!--
Target: /path/to/README.md
Created: 2025-12-10 14:30
Reason: Added authentication feature
-->"#;

    let meta = ArtifactMeta::from_header(header).unwrap();

    assert_eq!(meta.target, PathBuf::from("/path/to/README.md"));
    assert_eq!(meta.reason, "Added authentication feature");
    assert!(meta.based_on_patches.is_empty());
}

#[test]
fn parses_header_with_patches() {
    let header = r#"<!--
Target: /path/to/CLAUDE.md
Created: 2025-12-10 15:00
Reason: Updated conventions
Based on patches: 0001, 0002, 0003
-->"#;

    let meta = ArtifactMeta::from_header(header).unwrap();

    assert_eq!(meta.target, PathBuf::from("/path/to/CLAUDE.md"));
    assert_eq!(meta.based_on_patches, vec![1, 2, 3]);
}

#[test]
fn roundtrip_header() {
    let original = ArtifactMeta {
        target: PathBuf::from("/home/user/project/README.md"),
        created_at: DateTime::parse_from_rfc3339("2025-12-10T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc),
        reason: "Added new feature".to_string(),
        based_on_patches: vec![1, 5, 10],
    };

    let header = original.to_header();
    let parsed = ArtifactMeta::from_header(&header).unwrap();

    assert_eq!(original.target, parsed.target);
    assert_eq!(original.reason, parsed.reason);
    assert_eq!(original.based_on_patches, parsed.based_on_patches);
    // Note: created_at might differ slightly due to formatting precision
}

#[test]
fn returns_error_for_missing_delimiters() {
    let result = ArtifactMeta::from_header("No delimiters here");
    assert!(result.is_err());
}

#[test]
fn returns_error_for_missing_target() {
    let header = r#"<!--
Created: 2025-12-10 14:30
Reason: Some reason
-->"#;

    let result = ArtifactMeta::from_header(header);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Target"));
}

#[test]
fn returns_error_for_missing_created() {
    let header = r#"<!--
Target: /path/to/file.md
Reason: Some reason
-->"#;

    let result = ArtifactMeta::from_header(header);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Created"));
}

#[test]
fn returns_error_for_missing_reason() {
    let header = r#"<!--
Target: /path/to/file.md
Created: 2025-12-10 14:30
-->"#;

    let result = ArtifactMeta::from_header(header);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Reason"));
}
