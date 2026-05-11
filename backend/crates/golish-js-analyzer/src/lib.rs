//! Static analyzer for collected JavaScript bundles.
//!
//! Extracts API endpoint call-sites from JS source code, returning a
//! structured list of `{ method, path, params, auth, body_schema, ... }`
//! tuples that downstream pentest tools (e.g. `auth_probe`, IDOR/unauthorized
//! testing) can consume directly without paying LLM tokens to "read" each
//! bundle.
//!
//! ## Scope (P0)
//!
//! This first version uses **regex-based extraction**, not full AST parsing.
//! Rationale: production webpack/vite bundles are minified to the point that
//! swc AST traversal — while strictly more accurate — is dominated by
//! one-letter variable names and re-bound identifiers; the regex approach is
//! good enough for **plaintext** call sites (~90% of unminified or
//! sourcemap-resolved code) and ships in days rather than weeks.
//!
//! Future P1 will introduce an `swc_ecma_parser`-backed extractor as an
//! alternative `Extractor` impl, behind the same `extract_endpoints`
//! function signature.
//!
//! ## Recognized call patterns
//!
//! | Pattern | Example | Method | Path |
//! |---------|---------|--------|------|
//! | `fetch(url, init)` | `fetch('/api/users', { method: 'POST' })` | from init.method | literal |
//! | `axios.<verb>` | `axios.post('/api/orders', body)` | verb | literal |
//! | `axios(config)` | `axios({ url: '/x', method: 'PUT' })` | from config.method | literal |
//! | `$.ajax(config)` | `$.ajax({ url: '/y', type: 'POST' })` | from config.type | literal |
//! | `new Request(url, init)` | `new Request('/z', { method: 'DELETE' })` | from init.method | literal |
//!
//! ## What we DON'T cover (yet)
//! - Variable URL like `fetch(API_BASE + '/users')` — only literal-tail captured
//! - Template literals with interpolation: `` fetch(`/api/${id}`) `` — captured as `/api/${id}` raw
//! - Wrapped HTTP clients (custom `request()` helpers) — fall back to LLM
//! - Multi-line config objects whose fields span many lines — only first 200 chars after match are scanned
//!
//! These gaps are documented in the `Endpoint::confidence` field so callers
//! can filter low-confidence rows.

#![forbid(unsafe_code)]
#![deny(warnings)]
#![allow(clippy::result_large_err)]

use std::collections::HashSet;

use regex::Regex;
use serde::{Deserialize, Serialize};

mod ast_filter;
mod noise;
mod patterns;

pub use patterns::CallSiteKind;

/// Authentication hint inferred from the surrounding code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthHint {
    /// No `Authorization` / `X-Token` / cookie reference seen near the call site.
    None,
    /// `Authorization: Bearer ...` or `headers.Authorization` referenced.
    Bearer,
    /// Cookie-based session (`credentials: 'include'`, `withCredentials: true`).
    Cookie,
    /// Custom auth header (`X-Token`, `X-Api-Key`, etc.).
    Header,
    /// Code clearly references auth but pattern is ambiguous.
    Unknown,
}

/// Shape of the URL captured from source.
///
/// `auth_probe` (Stage 2) reads this to decide whether it can safely
/// substitute a path ID for cross-user IDOR testing:
/// - `Literal`: full path is a string constant — safe to test as-is.
/// - `Concatenated`: path is a prefix followed by `+ var` — the variable is
///   conventionally an ID; substitute by appending a different user's ID.
/// - `TemplateLiteral`: path contains `${...}` interpolation — substitute
///   by replacing inside the placeholder when its position is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UrlKind {
    /// `'/api/users'` — whole path is a string literal.
    #[default]
    Literal,
    /// `'/api/users/' + userId` — path prefix concatenated with a variable.
    Concatenated,
    /// `` `/api/users/${id}` `` — template literal with `${...}` placeholder.
    TemplateLiteral,
}

/// One extracted API call-site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    /// HTTP method, uppercase. Defaults to `GET` when the pattern does not
    /// expose one (e.g. plain `fetch(url)`).
    pub method: String,
    /// Raw URL/path string as it appears in source. May contain template
    /// interpolation markers (e.g. `/api/users/${id}`).
    pub path: String,
    /// Authentication hint inferred from nearby tokens.
    pub auth: AuthHint,
    /// Source file basename or relative path from the JS captures dir.
    pub source_file: String,
    /// 1-based line number where the call site starts.
    pub line: usize,
    /// 0.0..=1.0 — heuristic confidence score. Lower for variable URLs,
    /// template literals, or ambiguous wrapper clients.
    pub confidence: f32,
    /// Which family this endpoint was caught by.
    pub kind: CallSiteKind,
    /// Shape of the URL — see [`UrlKind`].
    #[serde(default)]
    pub url_kind: UrlKind,
    /// `true` when the path contains an ID-shaped segment (numeric, UUID,
    /// or 24+-byte hex) — strong signal that `auth_probe` cross-user
    /// scenario is applicable.
    #[serde(default)]
    pub has_path_params: bool,
    /// 0-based index (within `/`-split segments) of the first ID-shaped
    /// segment, when `has_path_params` is true. `None` for `Concatenated`
    /// or `TemplateLiteral` URLs whose ID position depends on runtime
    /// values not visible to the static analyzer.
    #[serde(default)]
    pub id_param_position: Option<usize>,
}

/// Aggregated extraction result for one or more JS files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractReport {
    /// Endpoints discovered. May contain duplicates across files; callers can
    /// dedupe by `(method, path)` if needed.
    pub endpoints: Vec<Endpoint>,
    /// Files that failed to scan or yielded zero hits, with a short reason.
    pub skipped: Vec<SkippedFile>,
    /// `(method, path)` pairs that were already recorded — useful for
    /// dedupe-aware metrics in downstream tools.
    pub unique: usize,
}

/// One file we couldn't extract anything useful from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedFile {
    pub file: String,
    pub reason: String,
}

/// Extract endpoints from a single JS source string.
///
/// `source_file` is recorded verbatim into each [`Endpoint::source_file`];
/// pass a basename or `host/port/js/foo.js`-style path that downstream tools
/// can map back to disk.
///
/// **P1 noise-filtering**: before running regex, comments (`//`, `/* */`)
/// and string literals are blanked-out (whitespace is preserved so byte
/// offsets / line numbers stay correct). This eliminates the regex
/// false-positives where a `fetch('/x')` snippet inside a comment or
/// `const docs = "..."` was getting picked up as a real call site.
pub fn extract_from_source(source_file: &str, content: &str) -> Vec<Endpoint> {
    // P2: pre-compute byte ranges of every tree-sitter-confirmed
    // call_expression / new_expression. Endpoints whose match offset
    // falls outside ALL of these ranges are filtered out as
    // false-positives at the very end. `None` here means tree-sitter
    // bailed (heavily-minified or corrupted source) — we degrade
    // gracefully and accept all regex matches.
    let ast_ranges = ast_filter::call_site_ranges(content);
    if let Some(false) = ast_filter::source_has_real_calls(content) {
        tracing::debug!(
            "[js-analyzer] {} parsed cleanly but contains no JS call expressions; \
             regex matches (if any) are highly suspect",
            source_file
        );
    }

    let scrubbed = noise::strip_noise(content);
    let scrubbed_str = scrubbed.as_str();
    let mut hits: Vec<(usize, Endpoint)> = Vec::new();

    // We pre-compile each pattern once per call. The regex compilation cost
    // is small (~1 ms total), so caching across calls is not worth the
    // global state. If profiling shows otherwise, hoist these to a
    // `lazy_static` later.
    let fetch_re = Regex::new(patterns::FETCH).expect("FETCH regex valid");
    let axios_verb_re = Regex::new(patterns::AXIOS_VERB).expect("AXIOS_VERB regex valid");
    let axios_config_re = Regex::new(patterns::AXIOS_CONFIG).expect("AXIOS_CONFIG regex valid");
    let jquery_re = Regex::new(patterns::JQUERY_AJAX).expect("JQUERY_AJAX regex valid");
    let new_request_re = Regex::new(patterns::NEW_REQUEST).expect("NEW_REQUEST regex valid");
    let fetch_concat_re = Regex::new(patterns::FETCH_CONCAT).expect("FETCH_CONCAT regex valid");
    let fetch_template_re =
        Regex::new(patterns::FETCH_TEMPLATE).expect("FETCH_TEMPLATE regex valid");

    // Order matters: run the concat / template patterns first and remember
    // their match offsets so the plain-`fetch` pattern doesn't re-emit the
    // same call site as a Literal. Otherwise `fetch('/api/' + id)` would
    // produce two endpoints — one Concatenated, one Literal with truncated
    // path.
    let mut shadowed_offsets: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for cap in fetch_concat_re.captures_iter(scrubbed_str) {
        let off = match cap.get(0) {
            Some(m) => {
                shadowed_offsets.insert(m.start());
                m.start()
            }
            None => continue,
        };
        if let Some(ep) = patterns::endpoint_from_fetch_concat(&cap, scrubbed_str, source_file) {
            hits.push((off, ep));
        }
    }
    for cap in fetch_template_re.captures_iter(scrubbed_str) {
        let off = match cap.get(0) {
            Some(m) => {
                shadowed_offsets.insert(m.start());
                m.start()
            }
            None => continue,
        };
        if let Some(ep) = patterns::endpoint_from_fetch_template(&cap, scrubbed_str, source_file) {
            hits.push((off, ep));
        }
    }
    for cap in fetch_re.captures_iter(scrubbed_str) {
        let off = match cap.get(0) {
            Some(m) => {
                if shadowed_offsets.contains(&m.start()) {
                    continue;
                }
                m.start()
            }
            None => continue,
        };
        if let Some(ep) = patterns::endpoint_from_fetch(&cap, scrubbed_str, source_file) {
            hits.push((off, ep));
        }
    }
    for cap in axios_verb_re.captures_iter(scrubbed_str) {
        let off = cap.get(0).map(|m| m.start()).unwrap_or(0);
        if let Some(ep) = patterns::endpoint_from_axios_verb(&cap, scrubbed_str, source_file) {
            hits.push((off, ep));
        }
    }
    for cap in axios_config_re.captures_iter(scrubbed_str) {
        let off = cap.get(0).map(|m| m.start()).unwrap_or(0);
        if let Some(ep) = patterns::endpoint_from_axios_config(&cap, scrubbed_str, source_file) {
            hits.push((off, ep));
        }
    }
    for cap in jquery_re.captures_iter(scrubbed_str) {
        let off = cap.get(0).map(|m| m.start()).unwrap_or(0);
        if let Some(ep) = patterns::endpoint_from_jquery(&cap, scrubbed_str, source_file) {
            hits.push((off, ep));
        }
    }
    for cap in new_request_re.captures_iter(scrubbed_str) {
        let off = cap.get(0).map(|m| m.start()).unwrap_or(0);
        if let Some(ep) = patterns::endpoint_from_new_request(&cap, scrubbed_str, source_file) {
            hits.push((off, ep));
        }
    }

    // P2 final filter: drop any hit whose byte offset is NOT inside a
    // tree-sitter-confirmed call_expression / new_expression node.
    // When `ast_ranges == None` (parser bailed) we keep everything —
    // graceful degradation.
    let endpoints: Vec<Endpoint> = if let Some(ref ranges) = ast_ranges {
        hits.into_iter()
            .filter(|(off, _)| ranges.contains_offset(*off))
            .map(|(_, ep)| ep)
            .collect()
    } else {
        hits.into_iter().map(|(_, ep)| ep).collect()
    };

    endpoints
}

/// Convenience: extract from many files and aggregate into a single report.
pub fn extract_from_files<I, S1, S2>(files: I) -> ExtractReport
where
    I: IntoIterator<Item = (S1, S2)>,
    S1: AsRef<str>,
    S2: AsRef<str>,
{
    let mut report = ExtractReport::default();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for (path, source) in files {
        let path_ref = path.as_ref();
        let source_ref = source.as_ref();
        let mut found = extract_from_source(path_ref, source_ref);

        if found.is_empty() {
            report.skipped.push(SkippedFile {
                file: path_ref.to_string(),
                reason: "no recognized HTTP call patterns".to_string(),
            });
            continue;
        }

        for ep in &found {
            if seen.insert((ep.method.clone(), ep.path.clone())) {
                report.unique += 1;
            }
        }
        report.endpoints.append(&mut found);
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_post_with_init() {
        let src = r#"
            fetch('/api/users', { method: 'POST', body: JSON.stringify(payload) });
        "#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1, "should catch one fetch call");
        assert_eq!(eps[0].method, "POST");
        assert_eq!(eps[0].path, "/api/users");
        assert_eq!(eps[0].kind, CallSiteKind::Fetch);
    }

    #[test]
    fn fetch_default_get() {
        let src = "fetch('/api/me')";
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].method, "GET");
        assert_eq!(eps[0].path, "/api/me");
    }

    #[test]
    fn axios_verb_helpers() {
        let src = r#"
            axios.get('/users');
            axios.post('/orders', body);
            axios.delete('/items/123');
        "#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 3);
        assert!(eps.iter().any(|e| e.method == "GET" && e.path == "/users"));
        assert!(eps
            .iter()
            .any(|e| e.method == "POST" && e.path == "/orders"));
        assert!(eps
            .iter()
            .any(|e| e.method == "DELETE" && e.path == "/items/123"));
    }

    #[test]
    fn axios_config_object() {
        let src = r#"
            axios({ url: '/api/login', method: 'PUT', data: payload });
        "#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].method, "PUT");
        assert_eq!(eps[0].path, "/api/login");
    }

    #[test]
    fn jquery_ajax() {
        let src = r#"
            $.ajax({ url: '/legacy', type: 'POST' });
        "#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].method, "POST");
        assert_eq!(eps[0].path, "/legacy");
    }

    #[test]
    fn new_request_constructor() {
        let src = r#"
            const req = new Request('/api/v2/data', { method: 'PATCH' });
        "#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].method, "PATCH");
        assert_eq!(eps[0].path, "/api/v2/data");
    }

    #[test]
    fn auth_bearer_inferred() {
        let src = r#"
            fetch('/secure', {
                method: 'GET',
                headers: { Authorization: 'Bearer ' + token }
            });
        "#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].auth, AuthHint::Bearer);
    }

    #[test]
    fn auth_cookie_inferred() {
        let src = r#"
            fetch('/with-cookie', { credentials: 'include' });
        "#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].auth, AuthHint::Cookie);
    }

    #[test]
    fn extract_from_files_dedupes_unique_count() {
        let files = [
            ("a.js", r#"fetch('/api/x', {method:'GET'})"#),
            ("b.js", r#"fetch('/api/x', {method:'GET'})"#),
            ("c.js", r#"axios.post('/api/y', body)"#),
        ];
        let report = extract_from_files(files.iter().map(|(p, s)| (*p, *s)));
        assert_eq!(report.endpoints.len(), 3, "all 3 occurrences listed");
        assert_eq!(report.unique, 2, "only 2 unique (method, path) pairs");
    }

    #[test]
    fn skipped_when_no_calls() {
        let report = extract_from_files(vec![("noise.js", "console.log('hi');")]);
        assert!(report.endpoints.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].reason.contains("no recognized"));
    }

    // ─── path-shape inference ───────────────────────────────────────────

    #[test]
    fn path_with_numeric_id_marks_path_params() {
        let src = r#"axios.get('/api/users/123')"#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert!(eps[0].has_path_params);
        // segments: ["api", "users", "123"] — 0-based, "123" is at idx 2
        assert_eq!(eps[0].id_param_position, Some(2));
        assert_eq!(eps[0].url_kind, UrlKind::Literal);
    }

    #[test]
    fn path_with_uuid_marks_path_params() {
        let src = r#"fetch('/items/550e8400-e29b-41d4-a716-446655440000')"#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert!(eps[0].has_path_params);
        assert_eq!(eps[0].id_param_position, Some(1));
    }

    #[test]
    fn path_with_mongo_objectid_marks_path_params() {
        let src = r#"fetch('/orders/507f1f77bcf86cd799439011')"#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert!(eps[0].has_path_params);
        assert_eq!(eps[0].id_param_position, Some(1));
    }

    #[test]
    fn path_without_id_segments_clears_flag() {
        let src = r#"fetch('/api/health')"#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert!(!eps[0].has_path_params);
        assert_eq!(eps[0].id_param_position, None);
    }

    // ─── concatenated URLs ──────────────────────────────────────────────

    #[test]
    fn fetch_concat_recognized_as_concatenated() {
        let src = r#"fetch('/api/users/' + userId)"#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1, "concat should not double-emit a literal too");
        assert_eq!(eps[0].url_kind, UrlKind::Concatenated);
        assert_eq!(eps[0].path, "/api/users/");
        assert!(eps[0].has_path_params);
        assert_eq!(eps[0].id_param_position, Some(2));
    }

    #[test]
    fn fetch_concat_with_method() {
        let src = r#"
            fetch('/api/orders/' + orderId, { method: 'DELETE' });
        "#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].method, "DELETE");
        assert_eq!(eps[0].url_kind, UrlKind::Concatenated);
    }

    // ─── template literal URLs ──────────────────────────────────────────

    #[test]
    fn fetch_template_recognized_as_template_literal() {
        let src = r#"fetch(`/api/users/${id}/posts`)"#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1, "template should not double-emit");
        assert_eq!(eps[0].url_kind, UrlKind::TemplateLiteral);
        assert!(eps[0].path.contains("${id}"));
        assert!(eps[0].has_path_params);
        // segments: ["api", "users", "${id}", "posts"] — `${` is at idx 2
        assert_eq!(eps[0].id_param_position, Some(2));
    }

    #[test]
    fn fetch_template_with_method_in_init() {
        let src = r#"
            fetch(`/api/items/${itemId}`, { method: 'PUT' })
        "#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].method, "PUT");
        assert_eq!(eps[0].url_kind, UrlKind::TemplateLiteral);
    }

    #[test]
    fn plain_fetch_still_marked_literal() {
        // Sanity: existing literal call sites must keep UrlKind::Literal
        // even after the concat/template patterns are introduced.
        let src = r#"fetch('/api/me')"#;
        let eps = extract_from_source("a.js", src);
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].url_kind, UrlKind::Literal);
        assert!(!eps[0].has_path_params);
    }
}
