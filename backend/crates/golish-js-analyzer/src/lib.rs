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
//! | `client.<verb>` | `Wr.post('/system/auth/login')` | verb | literal |
//! | `axios(config)` | `axios({ url: '/x', method: 'PUT' })` | from config.method | literal |
//! | `$.ajax(config)` | `$.ajax({ url: '/y', type: 'POST' })` | from config.type | literal |
//! | `new Request(url, init)` | `new Request('/z', { method: 'DELETE' })` | from init.method | literal |
//!
//! ## What we DON'T cover (yet)
//! - Variable URL like `fetch(API_BASE + '/users')` — only literal-tail captured
//! - Template literals with interpolation: `` fetch(`/api/${id}`) `` — captured as `/api/${id}` raw
//! - Opaque wrapper functions like `request('/x')` without an HTTP verb — fall back to LLM
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
mod signals;

pub use patterns::CallSiteKind;
pub use signals::{
    analyze_signals_from_files, analyze_signals_from_source, ConfigCandidate, ConfigKind,
    FrameworkCandidate, JsSignalReport, LibraryCandidate, RuleMatchCandidate, RuleMatchKind,
    RuleMatchSeverity, SecretCandidate, SecretKind,
};

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

/// Which extraction path produced an [`Endpoint`]: the deterministic regex
/// pass (default), HaE-style route/path rules, or the AI-assisted hybrid pass
/// (设计 2026-06-30-jsapi-ai-tools). `#[serde(default)]` on the field keeps old
/// persisted JSON (without this key) deserializing as [`EndpointSource::Regex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EndpointSource {
    /// Found by deterministic regex/AST analysis.
    #[default]
    Regex,
    /// Promoted mechanically from a HaE-style route/path candidate.
    Hae,
    /// Recovered by an LLM pass and anchored back to the source text.
    Ai,
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
    /// Which extraction path produced this endpoint. Defaults to
    /// [`EndpointSource::Regex`] for backward-compatible deserialization.
    #[serde(default)]
    pub source: EndpointSource,
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
    let http_client_verb_re =
        Regex::new(patterns::HTTP_CLIENT_VERB).expect("HTTP_CLIENT_VERB regex valid");
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
    for cap in http_client_verb_re.captures_iter(scrubbed_str) {
        let off = cap.get(0).map(|m| m.start()).unwrap_or(0);
        if let Some(ep) = patterns::endpoint_from_http_client_verb(&cap, scrubbed_str, source_file)
        {
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
#[path = "lib_tests.rs"]
mod tests;
