use super::*;

#[test]
fn creates_synthesis_input() {
    let input = ArtifactSynthesisInput::new(
        "# README".to_string(),
        vec!["feat: add feature".to_string()],
        "Goal: Add new feature".to_string(),
    );

    assert_eq!(input.existing_content, "# README");
    assert_eq!(input.patches_summary.len(), 1);
    assert_eq!(input.session_context, "Goal: Add new feature");
}

#[test]
fn builds_readme_prompt_with_patches() {
    let input = ArtifactSynthesisInput::new(
        "# My Project".to_string(),
        vec!["feat: add login".to_string(), "fix: fix bug".to_string()],
        "Session context".to_string(),
    );

    let prompt = input.build_readme_prompt();

    assert!(prompt.contains("# My Project"));
    assert!(prompt.contains("1. feat: add login"));
    assert!(prompt.contains("2. fix: fix bug"));
    assert!(prompt.contains("Session context"));
}

#[test]
fn builds_readme_prompt_without_patches() {
    let input = ArtifactSynthesisInput::new(
        "# My Project".to_string(),
        vec![],
        "Session context".to_string(),
    );

    let prompt = input.build_readme_prompt();

    assert!(prompt.contains("# My Project"));
    assert!(prompt.contains("No patches available"));
}

#[test]
fn builds_claude_md_prompt_with_patches() {
    let input = ArtifactSynthesisInput::new(
        "# CLAUDE.md\n\nInstructions".to_string(),
        vec!["refactor: update structure".to_string()],
        "Goal: Refactor".to_string(),
    );

    let prompt = input.build_claude_md_prompt();

    assert!(prompt.contains("# CLAUDE.md"));
    assert!(prompt.contains("1. refactor: update structure"));
    assert!(prompt.contains("Goal: Refactor"));
}
