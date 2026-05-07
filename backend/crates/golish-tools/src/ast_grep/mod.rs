//! AST-grep based code search and replace for Golish.
//!
//! This crate provides structural code search using AST patterns.
//! Unlike regex, it understands code structure and can match
//! syntactic patterns like function definitions, if statements, etc.

pub mod language;
mod replace_ops;
pub mod result;
pub mod tool;

#[cfg(test)]
mod tests;

pub use tool::{AstGrepReplaceTool, AstGrepTool};

use std::fs;
use std::panic;
use std::path::Path;

use anyhow::{Context, Result};
use ast_grep_language::{LanguageExt, SupportLang};
use walkdir::WalkDir;

pub use language::{detect_language, parse_language};
pub use replace_ops::{replace, replace_source};
pub use result::{ReplaceResult, Replacement, SearchMatch, SearchResult};

/// Search for AST patterns in source code.
pub fn search(
    workspace: &Path,
    pattern: &str,
    path: Option<&str>,
    language: Option<&str>,
) -> Result<SearchResult> {
    let target_path = match path {
        Some(p) => workspace.join(p),
        None => workspace.to_path_buf(),
    };

    let lang = language.and_then(parse_language);
    let mut result = SearchResult::new();

    if target_path.is_file() {
        search_file(&target_path, workspace, pattern, lang, &mut result)?;
        result.files_searched = 1;
    } else if target_path.is_dir() {
        for entry in WalkDir::new(&target_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let file_path = entry.path();
            let file_lang = lang.or_else(|| file_path.to_str().and_then(detect_language));

            if file_lang.is_some() {
                search_file(file_path, workspace, pattern, file_lang, &mut result)?;
                result.files_searched += 1;
            }
        }
    } else {
        anyhow::bail!("Path does not exist: {}", target_path.display());
    }

    Ok(result)
}

/// Search a single file for pattern matches.
fn search_file(
    file_path: &Path,
    workspace: &Path,
    pattern: &str,
    lang: Option<SupportLang>,
    result: &mut SearchResult,
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

    search_source_impl(&source, pattern, lang, &relative_path, result);

    Ok(())
}

/// Search source code string for pattern matches.
fn search_source_impl(
    source: &str,
    pattern: &str,
    lang: SupportLang,
    file_path: &str,
    result: &mut SearchResult,
) {
    let source = source.to_string();
    let pattern = pattern.to_string();
    let file_path = file_path.to_string();

    let search_result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let grep = lang.ast_grep(&source);
        let mut matches = Vec::new();

        for node_match in grep.root().find_all(pattern.as_str()) {
            let start = node_match.start_pos();
            let end = node_match.end_pos();
            let start_point = start.byte_point();
            let end_point = end.byte_point();

            matches.push(SearchMatch {
                file: file_path.clone(),
                line: start_point.0 + 1,
                column: start_point.1 + 1,
                text: node_match.text().to_string(),
                end_line: end_point.0 + 1,
                end_column: end_point.1 + 1,
            });
        }
        matches
    }));

    match search_result {
        Ok(matches) => {
            result.matches.extend(matches);
        }
        Err(_) => {
            result.error = Some(format!(
                "Invalid ast-grep pattern: '{}'. Use simple patterns like 'fn $NAME($$$ARGS)' for functions.",
                pattern
            ));
        }
    }
}

/// Search source code and return matches.
///
/// This is a convenience function for testing that searches a source string directly.
pub fn search_source(source: &str, pattern: &str, lang: SupportLang) -> Vec<SearchMatch> {
    let mut result = SearchResult::new();
    search_source_impl(source, pattern, lang, "<source>", &mut result);
    result.matches
}
