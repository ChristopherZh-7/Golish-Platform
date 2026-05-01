use super::*;

#[tokio::test]
async fn synthesize_readme_with_template_backend() {
    let config = ArtifactSynthesisConfig::default();
    let input = ArtifactSynthesisInput::new(
        "# Project".to_string(),
        vec!["feat: new feature".to_string()],
        "Goal: Add feature".to_string(),
    );

    let result = synthesize_readme(&config, &input).await.unwrap();

    assert_eq!(result.backend, "template");
    assert!(result.content.contains("# Project"));
    assert!(result.content.contains("## Recent Changes"));
}

#[tokio::test]
async fn synthesize_claude_md_with_template_backend() {
    let config = ArtifactSynthesisConfig::default();
    let input = ArtifactSynthesisInput::new(
        "# CLAUDE.md\n\nInstructions here.".to_string(),
        vec!["refactor: update structure".to_string()],
        "Session: Refactor codebase".to_string(),
    );

    let result = synthesize_claude_md(&config, &input).await.unwrap();

    assert_eq!(result.backend, "template");
    assert!(result.content.contains("# CLAUDE.md"));
    assert!(result.content.contains("## Session Notes"));
}

#[tokio::test]
async fn synthesize_readme_no_changes_when_no_patches() {
    let config = ArtifactSynthesisConfig::default();
    let input = ArtifactSynthesisInput::new(
        "# Project\n\nExisting content.".to_string(),
        vec![],
        "No-op session".to_string(),
    );

    let result = synthesize_readme(&config, &input).await.unwrap();

    // Template returns content unchanged when no patches
    assert_eq!(result.content, "# Project\n\nExisting content.");
}
