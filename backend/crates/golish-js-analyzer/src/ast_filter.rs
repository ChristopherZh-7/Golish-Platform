//! AST-grep based "real call site" probe — P2 (initial framework).
//!
//! `noise::strip_noise` already eliminates ~80% of regex false-positives.
//! This module is the framework for the remaining ~5% — the goal is to
//! parse the JS source with tree-sitter (via `ast-grep-language`) and
//! prove that real `call_expression` / `new_expression` nodes exist for
//! the call shapes the regex layer reported.
//!
//! ## Current scope (P2-init)
//!
//! Exposes [`source_has_real_calls`] — a coarse-grained existence check
//! that returns:
//! - `Some(true)`  → tree-sitter found at least one matching call
//! - `Some(false)` → parse succeeded but no call shapes at all
//! - `None`        → parse failed (heavily-minified / corrupt source)
//!
//! The orchestrator currently uses this only for **diagnostic logging**;
//! it does NOT yet drop endpoints based on it. A follow-up commit will
//! switch to byte-range filtering once the `ast_grep_core::Doc::range()`
//! plumbing is fully ironed out across the workspace.
//!
//! Even at this scope, having the ast-grep code path active is useful:
//! it surfaces malformed JS captures early (Sentry-style debug log) and
//! gives us an upgrade hook without a follow-up dependency change.

use ast_grep_language::{LanguageExt, SupportLang};
use std::panic;

/// Probe whether the source contains at least one tree-sitter-confirmed
/// call site (function call or `new` expression).
pub(crate) fn source_has_real_calls(source: &str) -> Option<bool> {
    let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let grep = SupportLang::JavaScript.ast_grep(source);
        let any_call = grep.root().find("$F($$$_)").is_some()
            || grep.root().find("new $F($$$_)").is_some();
        any_call
    }));
    match result {
        Ok(found) => Some(found),
        Err(_) => {
            tracing::debug!(
                "[js-analyzer] ast-grep parse panicked — falling back to regex-only"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_simple_fetch() {
        assert_eq!(source_has_real_calls("fetch('/x')"), Some(true));
    }

    #[test]
    fn detects_axios_post() {
        assert_eq!(
            source_has_real_calls("axios.post('/api/orders', body)"),
            Some(true)
        );
    }

    #[test]
    fn detects_new_request() {
        assert_eq!(
            source_has_real_calls("const r = new Request('/x', {});"),
            Some(true)
        );
    }

    #[test]
    fn empty_or_pure_data_has_no_calls() {
        assert_eq!(source_has_real_calls(""), Some(false));
        assert_eq!(source_has_real_calls("const x = 1;"), Some(false));
    }

    #[test]
    fn handles_unparseable_input_gracefully() {
        // Should never panic, regardless of return value.
        let _ = source_has_real_calls("\x00\x01 not js }}}");
    }
}
