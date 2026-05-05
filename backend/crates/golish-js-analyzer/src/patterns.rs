//! Regex patterns + per-pattern helpers that turn a `regex::Captures` into
//! an [`Endpoint`].
//!
//! Each pattern uses `(?m)` multi-line mode and is anchored to typical JS
//! call-site shapes. They're deliberately lenient about whitespace and
//! quote-style (single/double/backtick) but conservative about the general
//! shape so we minimize false positives like a stray string `"fetch"` in a
//! comment.

use regex::Captures;
use serde::{Deserialize, Serialize};

use crate::{AuthHint, Endpoint};

/// Family of HTTP-call call-site this endpoint was extracted from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallSiteKind {
    Fetch,
    AxiosVerb,
    AxiosConfig,
    JqueryAjax,
    NewRequest,
}

// ─────────────────────────────────────────────────────────────────────────
// Regex patterns
//
// Each captures **only the URL literal** in capture group 1; method (if any)
// is parsed from the surrounding window of the source string in the helper.
// We deliberately do NOT try to fit method+url+headers into a single pattern
// because that produces fragile multiline regex with high failure modes.
// ─────────────────────────────────────────────────────────────────────────

/// `fetch('/path' [...])` — greedy on first arg's quoted literal.
pub(crate) const FETCH: &str = r#"(?m)\bfetch\s*\(\s*[`'"]([^`'"]+)[`'"]"#;

/// `axios.<verb>('/path', ...)` — verb in capture group 1, url in 2.
pub(crate) const AXIOS_VERB: &str =
    r#"(?m)\baxios\s*\.\s*(get|post|put|patch|delete|head|options)\s*\(\s*[`'"]([^`'"]+)[`'"]"#;

/// `axios({ url: '/path', method: 'POST' })` — captures url; method parsed by helper.
pub(crate) const AXIOS_CONFIG: &str =
    r#"(?m)\baxios\s*\(\s*\{[^}]*?\burl\s*:\s*[`'"]([^`'"]+)[`'"]"#;

/// `$.ajax({ url: '/path', type: 'POST' })` or `jQuery.ajax(...)`.
pub(crate) const JQUERY_AJAX: &str =
    r#"(?m)(?:\$|jQuery)\s*\.\s*ajax\s*\(\s*\{[^}]*?\burl\s*:\s*[`'"]([^`'"]+)[`'"]"#;

/// `new Request('/path', { method: 'PUT' })`.
pub(crate) const NEW_REQUEST: &str =
    r#"(?m)\bnew\s+Request\s*\(\s*[`'"]([^`'"]+)[`'"]"#;

// ─────────────────────────────────────────────────────────────────────────
// Per-pattern helpers
// ─────────────────────────────────────────────────────────────────────────

pub(crate) fn endpoint_from_fetch(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    let path = cap.get(1)?.as_str().to_string();
    let match_start = cap.get(0)?.start();
    let init_window = window_after(source, match_start, 400);
    let method = method_from_init(init_window).unwrap_or_else(|| "GET".to_string());
    let auth = auth_from_window(init_window);
    Some(Endpoint {
        method,
        path,
        auth,
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.9,
        kind: CallSiteKind::Fetch,
    })
}

pub(crate) fn endpoint_from_axios_verb(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    let verb = cap.get(1)?.as_str().to_uppercase();
    let path = cap.get(2)?.as_str().to_string();
    let match_start = cap.get(0)?.start();
    Some(Endpoint {
        method: verb,
        path,
        auth: auth_from_window(window_after(source, match_start, 400)),
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.95, // verb is explicit, very high signal
        kind: CallSiteKind::AxiosVerb,
    })
}

pub(crate) fn endpoint_from_axios_config(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    let path = cap.get(1)?.as_str().to_string();
    let match_start = cap.get(0)?.start();
    let window = window_after(source, match_start, 400);
    // axios config uses "method" key, not "type"
    let method = method_from_init(window).unwrap_or_else(|| "GET".to_string());
    Some(Endpoint {
        method,
        path,
        auth: auth_from_window(window),
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.85,
        kind: CallSiteKind::AxiosConfig,
    })
}

pub(crate) fn endpoint_from_jquery(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    let path = cap.get(1)?.as_str().to_string();
    let match_start = cap.get(0)?.start();
    let window = window_after(source, match_start, 400);
    // jQuery uses both `type` (legacy) and `method` (>= 1.9.0)
    let method = method_from_jquery(window).unwrap_or_else(|| "GET".to_string());
    Some(Endpoint {
        method,
        path,
        auth: auth_from_window(window),
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.85,
        kind: CallSiteKind::JqueryAjax,
    })
}

pub(crate) fn endpoint_from_new_request(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    let path = cap.get(1)?.as_str().to_string();
    let match_start = cap.get(0)?.start();
    let window = window_after(source, match_start, 400);
    let method = method_from_init(window).unwrap_or_else(|| "GET".to_string());
    Some(Endpoint {
        method,
        path,
        auth: auth_from_window(window),
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.85,
        kind: CallSiteKind::NewRequest,
    })
}

// ─────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────

/// Returns the substring of `source` starting at `start` with length
/// `max_len`, byte-clamped at end-of-string.
fn window_after(source: &str, start: usize, max_len: usize) -> &str {
    let end = (start + max_len).min(source.len());
    // Be careful not to slice mid-char: walk back to a char boundary.
    let mut safe_end = end;
    while safe_end > start && !source.is_char_boundary(safe_end) {
        safe_end -= 1;
    }
    &source[start..safe_end]
}

/// Look for `method: '<VERB>'` (fetch / axios config / new Request init).
fn method_from_init(window: &str) -> Option<String> {
    static_search_method_key(window, "method")
}

/// Look for `type` first (jQuery legacy), then `method`.
fn method_from_jquery(window: &str) -> Option<String> {
    static_search_method_key(window, "type").or_else(|| static_search_method_key(window, "method"))
}

fn static_search_method_key(window: &str, key: &str) -> Option<String> {
    // Build a tiny one-shot regex; the key is only ever 'method' or 'type',
    // so escape risks are nil. This avoids dragging another lazy_static.
    let pat = format!(
        r#"\b{}\s*:\s*[`'"](GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)[`'"]"#,
        regex::escape(key)
    );
    let re = regex::RegexBuilder::new(&pat)
        .case_insensitive(true)
        .build()
        .ok()?;
    re.captures(window)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_uppercase()))
}

/// Heuristic auth detection inside the post-call-site window.
fn auth_from_window(window: &str) -> AuthHint {
    let lower = window.to_lowercase();
    if lower.contains("authorization") && lower.contains("bearer") {
        AuthHint::Bearer
    } else if lower.contains("credentials") && lower.contains("include")
        || lower.contains("withcredentials") && lower.contains("true")
    {
        AuthHint::Cookie
    } else if lower.contains("x-token")
        || lower.contains("x-api-key")
        || lower.contains("x-auth-token")
    {
        AuthHint::Header
    } else if lower.contains("authorization") {
        // Authorization header present but not bearer — still flagged.
        AuthHint::Unknown
    } else {
        AuthHint::None
    }
}

fn line_of(source: &str, byte_offset: usize) -> usize {
    1 + source[..byte_offset.min(source.len())]
        .bytes()
        .filter(|&b| b == b'\n')
        .count()
}
