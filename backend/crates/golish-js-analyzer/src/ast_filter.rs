//! AST-grep based "real call site" filter — P2 enhancement.
//!
//! `noise::strip_noise` already eliminates ~80% of regex false-positives.
//! This module covers the remaining ~5% by parsing the JS source with
//! tree-sitter (via `ast-grep-language`) and recording the byte ranges of
//! every `call_expression` and `new_expression` AST node. Endpoints whose
//! match offset doesn't fall inside one of those ranges are filtered out.
//!
//! ## Implementation note
//!
//! `ast_grep_core 0.40` exposes a generic `Node<'_, D>` whose `range()`
//! method requires a `Doc` bound that's awkward to thread through with the
//! parser's actual `D` type-parameter. The path of least resistance — and
//! the one already used by `golish-tools/src/ast_grep` — is the
//! `start_pos().byte_point()` API which yields `(line, col)` in BYTES.
//! We translate those points to flat byte offsets ourselves with one O(n)
//! scan that builds a `line_starts` index, then binary-search per node.
//!
//! Best-effort: if tree-sitter fails to parse the source (very rare;
//! corrupted/minified JS), [`call_site_ranges`] returns `None` and the
//! caller falls back to regex+noise-only filtering.

use ast_grep_language::{LanguageExt, SupportLang};
use std::panic;

/// Byte ranges (`start..end`, exclusive end) of every recognised "call
/// site" AST node in the source.
pub(crate) struct CallSiteRanges {
    /// Sorted by `start`.
    ranges: Vec<(usize, usize)>,
}

impl CallSiteRanges {
    fn from_unsorted(mut ranges: Vec<(usize, usize)>) -> Self {
        ranges.sort_by_key(|(s, _)| *s);
        Self { ranges }
    }

    /// Is `offset` inside any recorded range?
    pub(crate) fn contains_offset(&self, offset: usize) -> bool {
        match self.ranges.binary_search_by(|(s, _)| s.cmp(&offset)) {
            Ok(idx) => offset < self.ranges[idx].1,
            Err(idx) => {
                if idx == 0 {
                    false
                } else {
                    let (s, e) = self.ranges[idx - 1];
                    s <= offset && offset < e
                }
            }
        }
    }

    /// Number of recorded ranges (debug / tests only).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.ranges.len()
    }
}

/// Walk the JS AST and collect byte ranges of every node that matches
/// `kind == "call_expression" || kind == "new_expression"`.
///
/// Returns `None` if the parser bails. Callers treat `None` as
/// "filter is unavailable, accept all matches".
pub(crate) fn call_site_ranges(source: &str) -> Option<CallSiteRanges> {
    let line_starts = build_line_starts(source);

    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let grep = SupportLang::JavaScript.ast_grep(source);
        let mut ranges: Vec<(usize, usize)> = Vec::new();

        // `$F($$$_)` matches every call_expression — ast-grep's pattern
        // language treats `$F` as "any single node" so this is enough
        // to cover both bare-name and member-access call sites.
        for nm in grep.root().find_all("$F($$$_)") {
            if let Some(range) = node_match_byte_range(&nm, source, &line_starts) {
                ranges.push(range);
            }
        }
        for nm in grep.root().find_all("new $F($$$_)") {
            if let Some(range) = node_match_byte_range(&nm, source, &line_starts) {
                ranges.push(range);
            }
        }

        ranges
    }));

    match result {
        Ok(ranges) => Some(CallSiteRanges::from_unsorted(ranges)),
        Err(_) => {
            tracing::debug!(
                "[js-analyzer] ast-grep parse panicked — filter disabled for this file"
            );
            None
        }
    }
}

/// Diagnostic-only convenience used by `lib.rs` to emit a debug log when
/// regex matched something but tree-sitter saw zero call expressions.
pub(crate) fn source_has_real_calls(source: &str) -> Option<bool> {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let grep = SupportLang::JavaScript.ast_grep(source);
        grep.root().find("$F($$$_)").is_some() || grep.root().find("new $F($$$_)").is_some()
    }));
    result.ok()
}

/// Pre-compute the byte offset of each line's start so we can convert
/// `(line, col)` pairs from ast-grep to absolute byte offsets in O(log n).
fn build_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (idx, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(idx + 1);
        }
    }
    starts
}

fn line_col_to_byte(line: usize, col: usize, line_starts: &[usize], source_len: usize) -> usize {
    let base = line_starts.get(line).copied().unwrap_or(source_len);
    (base + col).min(source_len)
}

fn node_match_byte_range<D: ast_grep_core::Doc>(
    nm: &ast_grep_core::NodeMatch<'_, D>,
    source: &str,
    line_starts: &[usize],
) -> Option<(usize, usize)> {
    let start = nm.start_pos().byte_point();
    let end = nm.end_pos().byte_point();
    let s = line_col_to_byte(start.0, start.1, line_starts, source.len());
    let e = line_col_to_byte(end.0, end.1, line_starts, source.len());
    if e > s {
        Some((s, e))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_simple_fetch_call_range() {
        let src = "fetch('/api/users')";
        let ranges = call_site_ranges(src).expect("parse should succeed");
        assert!(ranges.len() >= 1);
        assert!(ranges.contains_offset(0));
        assert!(ranges.contains_offset(5)); // inside `('/api/users')`
        assert!(!ranges.contains_offset(src.len()));
    }

    #[test]
    fn rejects_offset_inside_doc_string() {
        let src = r#"const docs = "axios.get('/x')""#;
        let ranges = call_site_ranges(src).expect("parse should succeed");
        // Find the byte offset of "axios.get" inside the string literal.
        let needle = "axios.get";
        let off = src.find(needle).unwrap();
        assert!(
            !ranges.contains_offset(off),
            "offset inside a string literal must NOT be a call site"
        );
    }

    #[test]
    fn detects_new_request_constructor() {
        let src = "new Request('/api/v2', { method: 'PATCH' })";
        let ranges = call_site_ranges(src).expect("parse should succeed");
        assert!(ranges.contains_offset(0));
        // Past the closing paren is outside.
        assert!(!ranges.contains_offset(src.len()));
    }

    #[test]
    fn detects_axios_post() {
        let src = "axios.post('/api/orders', body)";
        let ranges = call_site_ranges(src).expect("parse should succeed");
        let off = src.find("axios.post").unwrap();
        assert!(ranges.contains_offset(off));
    }

    #[test]
    fn build_line_starts_handles_multiline_source() {
        let src = "a\nbb\nccc";
        let starts = build_line_starts(src);
        assert_eq!(starts, vec![0, 2, 5]);
    }

    #[test]
    fn line_col_to_byte_converts_correctly() {
        let src = "fetch\n('/x')";
        let starts = build_line_starts(src);
        // Line 1 col 0 = byte 6 (the `(`).
        assert_eq!(line_col_to_byte(1, 0, &starts, src.len()), 6);
    }

    #[test]
    fn handles_unparseable_input_gracefully() {
        let src = "\x00\x01 not js }}}";
        let _ = call_site_ranges(src); // no panic == pass
    }

    #[test]
    fn source_has_real_calls_diagnostic() {
        assert_eq!(source_has_real_calls("fetch('/x')"), Some(true));
        assert_eq!(source_has_real_calls("const x = 1;"), Some(false));
    }
}
