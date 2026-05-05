//! Static analyzer for collected JavaScript bundles.
//!
//! Extracts API endpoint call-sites from JS source code, returning a
//! structured list of `{ method, path, params, auth, body_schema, ... }`
//! tuples that downstream pentest tools (e.g. `auth_probe`, IDOR/未授权
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
pub fn extract_from_source(source_file: &str, content: &str) -> Vec<Endpoint> {
    let mut endpoints = Vec::new();

    // We pre-compile each pattern once per call. The regex compilation cost
    // is small (~1 ms total), so caching across calls is not worth the
    // global state. If profiling shows otherwise, hoist these to a
    // `lazy_static` later.
    let fetch_re = Regex::new(patterns::FETCH).expect("FETCH regex valid");
    let axios_verb_re = Regex::new(patterns::AXIOS_VERB).expect("AXIOS_VERB regex valid");
    let axios_config_re = Regex::new(patterns::AXIOS_CONFIG).expect("AXIOS_CONFIG regex valid");
    let jquery_re = Regex::new(patterns::JQUERY_AJAX).expect("JQUERY_AJAX regex valid");
    let new_request_re = Regex::new(patterns::NEW_REQUEST).expect("NEW_REQUEST regex valid");

    for cap in fetch_re.captures_iter(content) {
        if let Some(ep) = patterns::endpoint_from_fetch(&cap, content, source_file) {
            endpoints.push(ep);
        }
    }
    for cap in axios_verb_re.captures_iter(content) {
        if let Some(ep) = patterns::endpoint_from_axios_verb(&cap, content, source_file) {
            endpoints.push(ep);
        }
    }
    for cap in axios_config_re.captures_iter(content) {
        if let Some(ep) = patterns::endpoint_from_axios_config(&cap, content, source_file) {
            endpoints.push(ep);
        }
    }
    for cap in jquery_re.captures_iter(content) {
        if let Some(ep) = patterns::endpoint_from_jquery(&cap, content, source_file) {
            endpoints.push(ep);
        }
    }
    for cap in new_request_re.captures_iter(content) {
        if let Some(ep) = patterns::endpoint_from_new_request(&cap, content, source_file) {
            endpoints.push(ep);
        }
    }

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
        assert!(eps.iter().any(|e| e.method == "POST" && e.path == "/orders"));
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
        let files = vec![
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
}
