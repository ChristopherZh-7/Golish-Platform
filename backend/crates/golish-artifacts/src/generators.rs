//! Rule-Based Artifact Generation

/// Generate artifact content based on session context and patches
///
/// This is the rule-based generator - a simple template-based approach
/// that will be replaced with LLM-based generation in Phase 6.
pub fn generate_readme_update(
    current_readme: &str,
    session_context: &str,
    patch_summaries: &[String],
) -> String {
    // For now, just return the current README with a note about changes
    // This is a placeholder for the LLM-based generator
    let changes_section = if patch_summaries.is_empty() {
        String::new()
    } else {
        let changes = patch_summaries.join("\n- ");
        format!(
            "\n\n## Recent Changes\n\n- {}\n\n(Generated from session: {})",
            changes,
            session_context.lines().next().unwrap_or("unknown session")
        )
    };

    format!("{}{}", current_readme, changes_section)
}

/// Generate CLAUDE.md update based on session context
pub fn generate_claude_md_update(
    current_claude_md: &str,
    session_context: &str,
    patch_summaries: &[String],
) -> String {
    // Simple rule-based update - append conventions discovered
    let changes_section = if patch_summaries.is_empty() {
        String::new()
    } else {
        let changes = patch_summaries.join("\n- ");
        format!(
            "\n\n## Session Notes\n\n- {}\n\n(From session context)",
            changes
        )
    };

    // Check if there's new information that should be added
    let _ = session_context; // Will be used in LLM-based generation

    format!("{}{}", current_claude_md, changes_section)
}
