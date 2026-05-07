//! AST-grep based code replacement operations.

use std::fs;
use std::panic;
use std::path::Path;

use anyhow::{Context, Result};
use ast_grep_language::{LanguageExt, SupportLang};
use walkdir::WalkDir;

use super::{detect_language, parse_language, ReplaceResult, Replacement};

/// Replace AST patterns in source code.
///
/// # Arguments
///
/// * `workspace` - The workspace root directory
/// * `pattern` - AST pattern to match (e.g., "console.log($MSG)")
/// * `replacement` - Replacement template (e.g., "logger.info($MSG)")
/// * `path` - Relative path to modify (file or directory)
/// * `language` - Optional language hint. Auto-detected from file extension if not provided.
///
/// # Returns
///
/// A `ReplaceResult` containing information about the replacements made.
pub fn replace(
    workspace: &Path,
    pattern: &str,
    replacement: &str,
    path: &str,
    language: Option<&str>,
) -> Result<ReplaceResult> {
    let target_path = workspace.join(path);
    let lang = language.and_then(parse_language);
    let mut result = ReplaceResult::new();

    if target_path.is_file() {
        replace_file(
            &target_path,
            workspace,
            pattern,
            replacement,
            lang,
            &mut result,
        )?;
    } else if target_path.is_dir() {
        for entry in WalkDir::new(&target_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let file_path = entry.path();
            let file_lang = lang.or_else(|| file_path.to_str().and_then(detect_language));

            if file_lang.is_some() {
                replace_file(
                    file_path,
                    workspace,
                    pattern,
                    replacement,
                    file_lang,
                    &mut result,
                )?;
            }
        }
    } else {
        anyhow::bail!("Path does not exist: {}", target_path.display());
    }

    Ok(result)
}

/// Replace patterns in a single file.
fn replace_file(
    file_path: &Path,
    workspace: &Path,
    pattern: &str,
    replacement: &str,
    lang: Option<SupportLang>,
    result: &mut ReplaceResult,
) -> Result<()> {
    let lang = match lang {
        Some(l) => l,
        None => match file_path.to_str().and_then(detect_language) {
            Some(l) => l,
            None => return Ok(()),
        },
    };

    let source = fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

    let relative_path = file_path
        .strip_prefix(workspace)
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();

    let (new_source, changes, error) =
        replace_source_impl(&source, pattern, replacement, lang, &relative_path);

    if let Some(err) = error {
        tracing::warn!("ast-grep replace error: {}", err);
        return Ok(());
    }

    if !changes.is_empty() {
        fs::write(file_path, &new_source)
            .with_context(|| format!("Failed to write file: {}", file_path.display()))?;

        result.files_modified.push(relative_path);
        result.replacements_count += changes.len();
        result.changes.extend(changes);
    }

    Ok(())
}

/// Replace patterns in source code and return the new source, changes, and optional error.
fn replace_source_impl(
    source: &str,
    pattern: &str,
    replacement: &str,
    lang: SupportLang,
    file_path: &str,
) -> (String, Vec<Replacement>, Option<String>) {
    let source = source.to_string();
    let pattern = pattern.to_string();
    let replacement = replacement.to_string();
    let file_path = file_path.to_string();

    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let grep = lang.ast_grep(&source);

        let mut changes = Vec::new();
        let mut new_source = source.clone();

        let mut matches: Vec<_> = grep.root().find_all(pattern.as_str()).collect();

        matches.sort_by_key(|m| std::cmp::Reverse(m.range().start));

        for node_match in matches {
            let original = node_match.text().to_string();
            let start = node_match.start_pos();
            let start_point = start.byte_point();
            let range = node_match.range();

            let replaced = generate_replacement(&node_match, &replacement, lang);

            new_source.replace_range(range.start..range.end, &replaced);

            changes.push(Replacement {
                file: file_path.clone(),
                line: start_point.0 + 1,
                original,
                replacement: replaced,
            });
        }

        changes.reverse();

        (new_source, changes)
    }));

    match result {
        Ok((new_source, changes)) => (new_source, changes, None),
        Err(_) => (
            source,
            Vec::new(),
            Some(format!(
                "Invalid ast-grep pattern: '{}'. Use simple patterns like 'fn $NAME($$$ARGS)' for functions.",
                pattern
            )),
        ),
    }
}

/// Generate replacement text by substituting captured meta-variables.
fn generate_replacement<D: ast_grep_core::Doc>(
    node_match: &ast_grep_core::NodeMatch<D>,
    replacement: &str,
    _lang: SupportLang,
) -> String {
    let env = node_match.get_env();

    let mut i = 0;
    let chars: Vec<char> = replacement.chars().collect();
    let mut new_result = String::new();

    while i < chars.len() {
        if chars[i] == '$' {
            if i + 2 < chars.len() && chars[i + 1] == '$' && chars[i + 2] == '$' {
                let start = i + 3;
                let end = find_var_end(&chars, start);
                if end > start {
                    let var_name: String = chars[start..end].iter().collect();
                    let nodes = env.get_multiple_matches(&var_name);
                    let text: String = nodes
                        .iter()
                        .map(|n| n.text().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    new_result.push_str(&text);
                    i = end;
                    continue;
                }
            }
            let start = i + 1;
            let end = find_var_end(&chars, start);
            if end > start {
                let var_name: String = chars[start..end].iter().collect();
                if let Some(node) = env.get_match(&var_name) {
                    new_result.push_str(&node.text());
                    i = end;
                    continue;
                }
            }
        }
        new_result.push(chars[i]);
        i += 1;
    }

    new_result
}

/// Find the end of a variable name (alphanumeric + underscore)
fn find_var_end(chars: &[char], start: usize) -> usize {
    let mut end = start;
    while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    end
}

/// Replace patterns in source code and return the result.
///
/// This is a convenience function for testing.
pub fn replace_source(source: &str, pattern: &str, replacement: &str, lang: SupportLang) -> String {
    let (new_source, _, _) = replace_source_impl(source, pattern, replacement, lang, "<source>");
    new_source
}
