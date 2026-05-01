//! Rule-based (no-LLM) state synthesizer.
//!
//! Maintains the `## Changes` section of `state.md` by appending newly
//! touched files. Used as the default backend so projects without LLM
//! credentials still get a usable session diary.

use anyhow::Result;

use super::{StateSynthesisInput, StateSynthesisResult, StateSynthesizer};

/// Template-based state synthesizer (rule-based updates, no LLM).
pub struct TemplateStateSynthesizer;

impl TemplateStateSynthesizer {
    pub fn new() -> Self {
        Self
    }

    /// Update the Changes section with new files.
    fn update_changes_section(state: &str, files: &[String]) -> String {
        if files.is_empty() {
            return state.to_string();
        }

        let new_entries: Vec<String> = files
            .iter()
            .map(|f| {
                // Extract just the filename if it contains diff info.
                let path = f.lines().next().unwrap_or(f);
                format!("- `{}`", path)
            })
            .collect();

        if let Some(changes_idx) = state.find("## Changes") {
            let (before, after_changes) = state.split_at(changes_idx);

            // Find the end of the Changes section (next ## or end of string).
            let changes_content_start = after_changes.find('\n').unwrap_or(after_changes.len());
            let rest = &after_changes[changes_content_start..];

            let next_section = rest.find("\n## ").map(|i| i + 1);
            let (changes_body, remainder) = match next_section {
                Some(idx) => rest.split_at(idx),
                None => (rest, ""),
            };

            let updated_changes = if changes_body.contains("(none yet)") {
                new_entries.join("\n")
            } else {
                let existing: std::collections::HashSet<_> = changes_body
                    .lines()
                    .filter(|l| l.starts_with("- "))
                    .collect();

                let mut all_changes: Vec<String> = changes_body
                    .lines()
                    .filter(|l| l.starts_with("- "))
                    .map(|s| s.to_string())
                    .collect();

                for entry in &new_entries {
                    if !existing.contains(entry.as_str()) {
                        all_changes.push(entry.clone());
                    }
                }

                all_changes.join("\n")
            };

            format!(
                "{}## Changes\n{}\n{}",
                before,
                updated_changes,
                remainder.trim_start()
            )
        } else {
            // No Changes section found, append one.
            format!("{}\n## Changes\n{}\n", state, new_entries.join("\n"))
        }
    }
}

impl Default for TemplateStateSynthesizer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl StateSynthesizer for TemplateStateSynthesizer {
    async fn synthesize_state(&self, input: &StateSynthesisInput) -> Result<StateSynthesisResult> {
        let state_body = Self::update_changes_section(&input.current_state, &input.files);

        Ok(StateSynthesisResult {
            state_body,
            backend: "template".to_string(),
        })
    }

    fn backend_name(&self) -> &'static str {
        "template"
    }
}
