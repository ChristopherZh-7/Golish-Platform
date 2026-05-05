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

use crate::{AuthHint, Endpoint, UrlKind};

/// Family of HTTP-call call-site this endpoint was extracted from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// `fetch('/api/users/' + id, ...)` — captures the literal prefix when
/// the URL is built by string concatenation. The `+` after the closing
/// quote is the disambiguator that prevents this from re-matching plain
/// `FETCH` literal call sites.
pub(crate) const FETCH_CONCAT: &str =
    r#"(?m)\bfetch\s*\(\s*[`'"]([^`'"]+)[`'"]\s*\+"#;

/// `` fetch(`/api/users/${id}`, ...) `` — backtick-quoted template literal
/// with at least one `${...}` placeholder. We capture the raw template
/// body (placeholders preserved) so callers see the same text the model
/// would.
pub(crate) const FETCH_TEMPLATE: &str =
    r#"(?m)\bfetch\s*\(\s*`([^`]*\$\{[^`]*)`"#;

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
    let (has_path_params, id_param_position) = analyze_path(&path);
    Some(Endpoint {
        method,
        path,
        auth,
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.9,
        kind: CallSiteKind::Fetch,
        url_kind: UrlKind::Literal,
        has_path_params,
        id_param_position,
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
    let (has_path_params, id_param_position) = analyze_path(&path);
    Some(Endpoint {
        method: verb,
        path,
        auth: auth_from_window(window_after(source, match_start, 400)),
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.95, // verb is explicit, very high signal
        kind: CallSiteKind::AxiosVerb,
        url_kind: UrlKind::Literal,
        has_path_params,
        id_param_position,
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
    let (has_path_params, id_param_position) = analyze_path(&path);
    Some(Endpoint {
        method,
        path,
        auth: auth_from_window(window),
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.85,
        kind: CallSiteKind::AxiosConfig,
        url_kind: UrlKind::Literal,
        has_path_params,
        id_param_position,
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
    let (has_path_params, id_param_position) = analyze_path(&path);
    Some(Endpoint {
        method,
        path,
        auth: auth_from_window(window),
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.85,
        kind: CallSiteKind::JqueryAjax,
        url_kind: UrlKind::Literal,
        has_path_params,
        id_param_position,
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
    let (has_path_params, id_param_position) = analyze_path(&path);
    Some(Endpoint {
        method,
        path,
        auth: auth_from_window(window),
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.85,
        kind: CallSiteKind::NewRequest,
        url_kind: UrlKind::Literal,
        has_path_params,
        id_param_position,
    })
}

/// `fetch('/prefix/' + id)` — concatenation. Path is the literal prefix;
/// id position is the trailing slot (one past the prefix's last segment).
///
/// Position counting is consistent with [`analyze_path`]: leading and
/// trailing `/` are stripped before splitting, then segments are 0-based.
pub(crate) fn endpoint_from_fetch_concat(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    let prefix = cap.get(1)?.as_str().to_string();
    let match_start = cap.get(0)?.start();
    let window = window_after(source, match_start, 400);
    let method = method_from_init(window).unwrap_or_else(|| "GET".to_string());
    // After trimming, count existing segments — the runtime variable
    // becomes segment `seg_count` (0-based), which is the slot right
    // after the last existing one.
    //   "/api/users/" -> trim -> "api/users" -> 2 segments -> id slot = 2
    //   "/api/users"  -> trim -> "api/users" -> 2 segments -> id slot = 2
    let trimmed = prefix.trim_start_matches('/').trim_end_matches('/');
    let seg_count = if trimmed.is_empty() {
        0
    } else {
        trimmed.split('/').count()
    };
    Some(Endpoint {
        method,
        path: prefix,
        auth: auth_from_window(window),
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.7, // concat URLs are inherently less certain
        kind: CallSiteKind::Fetch,
        url_kind: UrlKind::Concatenated,
        has_path_params: true, // concat necessarily has a runtime variable
        id_param_position: Some(seg_count),
    })
}

/// `` fetch(`/path/${id}`) `` — template literal. Path keeps `${...}`
/// markers. id position is the 0-based index (after trimming surrounding
/// `/`) of the first segment containing `${`.
pub(crate) fn endpoint_from_fetch_template(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    let template_body = cap.get(1)?.as_str().to_string();
    let match_start = cap.get(0)?.start();
    let window = window_after(source, match_start, 400);
    let method = method_from_init(window).unwrap_or_else(|| "GET".to_string());
    let trimmed = template_body
        .trim_start_matches('/')
        .trim_end_matches('/');
    let id_pos = trimmed.split('/').position(|seg| seg.contains("${"));
    Some(Endpoint {
        method,
        path: template_body,
        auth: auth_from_window(window),
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.75, // template literal is more structured than concat
        kind: CallSiteKind::Fetch,
        url_kind: UrlKind::TemplateLiteral,
        has_path_params: true,
        id_param_position: id_pos,
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
///
/// Recognizes both pretty-printed and minified forms (e.g. `true` → `!0`,
/// `false` → `!1` from terser/uglify output).
fn auth_from_window(window: &str) -> AuthHint {
    let lower = window.to_lowercase();
    if lower.contains("authorization") && lower.contains("bearer") {
        AuthHint::Bearer
    } else if (lower.contains("credentials") && lower.contains("include"))
        || (lower.contains("withcredentials")
            && (lower.contains("true") || lower.contains("!0")))
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

/// Heuristic: detect path segments that look like an ID and return the
/// 0-based position of the first such segment.
///
/// Recognized ID shapes:
/// - Pure digits (`\d+`) — e.g. `/api/users/123`
/// - UUID v4-ish (`8-4-4-4-12` hex with dashes)
/// - 24+-byte hex string — common for Mongo ObjectIds and similar
///
/// Strips leading/trailing slashes before splitting so the position counts
/// from the first non-empty segment.
pub(crate) fn analyze_path(path: &str) -> (bool, Option<usize>) {
    let trimmed = path.trim_start_matches('/').trim_end_matches('/');
    if trimmed.is_empty() {
        return (false, None);
    }
    for (idx, seg) in trimmed.split('/').enumerate() {
        if is_id_shaped(seg) {
            return (true, Some(idx));
        }
    }
    (false, None)
}

fn is_id_shaped(seg: &str) -> bool {
    if seg.is_empty() {
        return false;
    }
    // Pure digits — most common ID shape.
    if seg.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    // UUID 8-4-4-4-12 hex with dashes.
    if seg.len() == 36 {
        let bytes = seg.as_bytes();
        let dash_positions = [8usize, 13, 18, 23];
        let dashes_ok = dash_positions.iter().all(|&p| bytes[p] == b'-');
        let hex_ok = bytes
            .iter()
            .enumerate()
            .all(|(i, &b)| dash_positions.contains(&i) || b.is_ascii_hexdigit());
        if dashes_ok && hex_ok {
            return true;
        }
    }
    // 24+-byte hex string (Mongo ObjectId, SHA-1 prefix, etc.).
    if seg.len() >= 24 && seg.bytes().all(|b| b.is_ascii_hexdigit()) {
        return true;
    }
    false
}
