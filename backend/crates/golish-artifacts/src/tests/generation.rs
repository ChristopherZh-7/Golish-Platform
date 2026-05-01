use super::*;

#[test]
fn generates_readme_with_changes() {
    let current = "# Project\n\nA cool project.";
    let context = "Goal: Add authentication";
    let patches = vec!["feat(auth): add login".to_string()];

    let result = generate_readme_update(current, context, &patches);

    assert!(result.contains("# Project"));
    assert!(result.contains("## Recent Changes"));
    assert!(result.contains("feat(auth): add login"));
}

#[test]
fn generates_readme_without_changes() {
    let current = "# Project\n\nA cool project.";
    let context = "Goal: Review code";
    let patches: Vec<String> = vec![];

    let result = generate_readme_update(current, context, &patches);

    assert_eq!(result, current);
}

#[test]
fn generates_claude_md_with_changes() {
    let current = "# CLAUDE.md\n\nInstructions.";
    let context = "Session context here";
    let patches = vec!["Added new convention".to_string()];

    let result = generate_claude_md_update(current, context, &patches);

    assert!(result.contains("# CLAUDE.md"));
    assert!(result.contains("## Session Notes"));
}
