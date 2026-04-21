//! LLM Prompt Templates for Artifact Synthesis

/// System prompt for README.md generation
pub const README_SYSTEM_PROMPT: &str = r#"You are a technical writer updating a README.md file based on recent code changes.

## Your Task
Analyze the provided patches and update the README to accurately reflect the current state of the project.

## Guidelines
- Update relevant sections only; preserve existing structure and content that is still accurate
- Focus on user-facing changes: new features, changed APIs, updated usage instructions
- Be concise and clear; avoid unnecessary verbosity
- Maintain the existing writing style and tone
- Do not add boilerplate or placeholder sections
- If the changes are purely internal/refactoring with no user impact, make minimal or no changes

## Output Format
Return ONLY the updated README.md content, no explanations or markdown code blocks.
The output should be ready to save directly as README.md."#;

/// User prompt template for README.md generation
pub const README_USER_PROMPT: &str = r#"Update this README.md based on recent changes.

## Current README.md
```markdown
{existing_content}
```

## Recent Changes (patch summaries)
{patches_summary}

## Session Context
{session_context}

Generate the updated README.md content."#;

/// System prompt for CLAUDE.md generation
pub const CLAUDE_MD_SYSTEM_PROMPT: &str = r#"You are updating a CLAUDE.md file (AI assistant instructions) based on recent code changes.

## About CLAUDE.md
CLAUDE.md provides context and conventions for AI assistants working on this codebase. It typically includes:
- Project overview and architecture
- Commands and workflows
- Code conventions and patterns
- Important files and their purposes

## Your Task
Update CLAUDE.md to reflect new conventions, patterns, or architecture discovered in the patches.

## Guidelines
- Add new commands or workflows if introduced
- Update architecture sections if structure changed
- Add new conventions discovered from the code changes
- Preserve existing accurate content
- Keep instructions actionable and specific
- Do not remove existing content unless it's clearly outdated

## Output Format
Return ONLY the updated CLAUDE.md content, no explanations or markdown code blocks.
The output should be ready to save directly as CLAUDE.md."#;

/// User prompt template for CLAUDE.md generation
pub const CLAUDE_MD_USER_PROMPT: &str = r#"Update this CLAUDE.md based on recent changes.

## Current CLAUDE.md
```markdown
{existing_content}
```

## Recent Changes (patch summaries)
{patches_summary}

## Session Context
{session_context}

Generate the updated CLAUDE.md content."#;

// =============================================================================
// Artifact Synthesis Backend
