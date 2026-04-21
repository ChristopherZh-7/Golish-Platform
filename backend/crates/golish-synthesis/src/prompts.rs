//! LLM Prompt Templates

/// System prompt for LLM-based commit message generation
pub const COMMIT_MESSAGE_SYSTEM_PROMPT: &str = r#"You are a commit message generator. Generate concise, conventional commit messages from git diffs.

## Guidelines
- Use conventional commit format: type(scope): description
- Types: feat, fix, refactor, docs, test, chore, perf, style, build, ci
- First line (subject) must be <= 72 characters
- Body explains what changed and why (not how)
- Be specific but concise

## Conventional Commit Types
- feat: A new feature for the user
- fix: A bug fix for the user
- refactor: Code restructuring without behavior change
- docs: Documentation only changes
- test: Adding or updating tests
- chore: Maintenance tasks, dependencies, tooling
- perf: Performance improvements
- style: Code style/formatting changes
- build: Build system or external dependency changes
- ci: CI configuration changes

## Format
```
type(scope): short description

Optional body explaining the change in more detail.
What was changed and why (not how).
```

Return ONLY the commit message, no additional text or markdown formatting."#;

/// User prompt template for commit message generation
pub const COMMIT_MESSAGE_USER_PROMPT: &str = r#"Generate a commit message for the following changes:

## Session Context
{context}

## Git Diff
```diff
{diff}
```

## Files Changed
{files}

Generate a conventional commit message for these changes."#;

/// System prompt for LLM-based state.md updates
pub const STATE_UPDATE_SYSTEM_PROMPT: &str = r#"
You maintain a session state file that tracks goals and change rationale for an AI coding agent.

The end goal is to create a comprehensive overview of this coding sessions goals, intents, and the changes that were made.

## Rules
- Goals: Extract the user's intent and goals from the user prompts. Provide a breakdown of their goals and intents as a list of bullet points.
- Changes: Each file change gets a reason. Why was this change made?

## Output
Return the complete updated state.md file. Nothing else.
Use the format below.

<format>
# Session State
Updated: {timestamp}

## Goals
{user's goals and intents}

## Changes
- `{file path}` — {why this change was made}
- `{file path}` — {why this change was made}
</format>
"#;

/// User prompt template for state.md updates
pub const STATE_UPDATE_USER_PROMPT: &str = r#"<current_state>
{current_state}
</current_state>

<event>
type: {event_type}
content: {event_details}
files: {files}
</event>"#;

/// System prompt for LLM-based session title generation
pub const SESSION_TITLE_SYSTEM_PROMPT: &str = r#"Generate a concise, descriptive title for a coding session.

## Guidelines
- Be specific about what was accomplished or attempted
- Use 3-8 words
- Focus on the main goal or feature
- Avoid generic phrases like "coding session" or "working on"
- Use title case

## Examples
Good titles:
- "Implement User Authentication"
- "Fix Memory Leak in Parser"
- "Add Dark Mode Support"
- "Refactor Database Layer"
- "Setup CI/CD Pipeline"

Bad titles:
- "Coding session" (too generic)
- "Working on the project" (too vague)
- "Bug fixes and improvements" (not specific)

## Handling Vague Inputs
If the user's request is vague (like "hello" or "hi"), generate a title based on:
- Modified files mentioned in the session state
- Any patterns in filenames (e.g., config files → "Update Configuration")
- If nothing specific, use "New Coding Session"

CRITICAL: You MUST return ONLY the title. Never ask questions. Never explain. Just output the title text."#;
