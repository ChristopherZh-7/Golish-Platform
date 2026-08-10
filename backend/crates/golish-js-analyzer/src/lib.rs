//! Static analyzer for collected JavaScript bundles.
//!
//! Extracts API endpoint call-sites from JS source code, returning a
//! structured list of `{ method, path, params, auth, body_schema, ... }`
//! tuples that bounded anonymous-access review and later Candidate verification
//! can consume directly without paying LLM tokens to "read" each bundle.
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
use std::fmt::Write as _;

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
/// Downstream consumers use this to distinguish concrete URLs from unresolved
/// runtime templates. A shape hint is never permission to invent or substitute
/// path values: anonymous-access probes require an exact safe endpoint, and
/// Candidate verification replays only the frozen observation binding.
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
    /// or 24+-byte hex). This is a Candidate-analysis signal, not permission
    /// for a caller to guess or substitute identifiers.
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

/// Byte-precise location of deterministic source evidence.
///
/// The end offset is exclusive. For regex-backed call candidates this span
/// covers the matched callee and URL literal, which is enough to distinguish
/// multiple calls on one minified line without persisting surrounding source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    /// 1-based source line.
    pub line: usize,
    /// 0-based byte column within `line`.
    pub column: usize,
}

/// Deterministic call-site facts that are intentionally kept separate from
/// [`Endpoint`] so existing consumers and persisted endpoint JSON remain
/// backward compatible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallSiteContext {
    /// Normalized callee label such as `fetch`, `axios.post`, or `admin.get`.
    pub callee: String,
    /// Receiver identifier when the call has one (`admin` in `admin.get`).
    pub receiver: Option<String>,
    pub span: SourceSpan,
}

/// Closed adapter family used to interpret one AST-confirmed call site.
///
/// `Raw` deliberately carries no inferred argument/config semantics. This is
/// the fail-closed result for custom wrappers that have a URL-shaped first
/// argument but are not one of the adapters understood by this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CallAdapter {
    #[default]
    Raw,
    Fetch,
    Axios,
    Request,
    XmlHttpRequest,
    JQuery,
    Graphql,
    WebSocket,
    EventSource,
}

/// Value-free location assigned to a parameter name at one exact call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterLocation {
    Path,
    Query,
    Body,
    Form,
    Header,
    GraphqlVariable,
    Unknown,
}

/// Static type shape. Dynamic expressions remain [`ParameterValueType::Unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ParameterValueType {
    String,
    Number,
    Boolean,
    Object,
    Array,
    Null,
    #[default]
    Unknown,
}

/// A field-name fact extracted from one AST-confirmed call node.
///
/// This type intentionally has no value, preview, or value hash field. Values
/// are only inspected transiently to derive the coarse static type above.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterFact {
    pub name: String,
    pub location: ParameterLocation,
    pub value_type: ParameterValueType,
}

/// Semantic role of a top-level call argument. The argument expression itself
/// is never retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentRole {
    Url,
    Config,
    Body,
    Form,
    GraphqlDocument,
    GraphqlVariables,
    Unknown,
}

/// Value-free top-level argument shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgumentFact {
    pub index: usize,
    pub role: ArgumentRole,
    pub value_type: ParameterValueType,
    pub dynamic: bool,
}

/// Value-free top-level config field shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigFact {
    pub name: String,
    pub value_type: ParameterValueType,
}

/// GraphQL operation metadata is schema identity rather than an argument
/// value, so it is safe to retain alongside variable names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphqlOperationKind {
    Query,
    Mutation,
    Subscription,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphqlOperationFact {
    pub kind: GraphqlOperationKind,
    pub name: Option<String>,
}

/// One raw endpoint plus the source-local call-site evidence needed by a
/// downstream contextual resolver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointCandidate {
    /// Stable source-local identity. It hashes only source identity, exact
    /// byte span and callee — never argument values or secret material.
    #[serde(default)]
    pub candidate_id: String,
    pub endpoint: Endpoint,
    pub call: CallSiteContext,
    #[serde(default)]
    pub adapter: CallAdapter,
    #[serde(default)]
    pub arguments: Vec<ArgumentFact>,
    #[serde(default)]
    pub config: Vec<ConfigFact>,
    #[serde(default)]
    pub parameters: Vec<ParameterFact>,
    #[serde(default)]
    pub graphql_operation: Option<GraphqlOperationFact>,
}

/// Candidate-preserving counterpart to [`ExtractReport`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CandidateExtractReport {
    pub candidates: Vec<EndpointCandidate>,
    pub skipped: Vec<SkippedFile>,
    /// Unique raw `(method, path)` pairs. Occurrences remain in `candidates`.
    pub unique: usize,
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
    extract_candidates_from_source(source_file, content)
        .into_iter()
        .filter(legacy_candidate_visible)
        .map(|candidate| candidate.endpoint)
        .collect()
}

/// Extract endpoint call-sites while preserving receiver and byte-span facts.
///
/// This is additive to [`extract_from_source`]. Downstream resolvers should
/// consume this API when raw `(method, path)` is insufficient to distinguish
/// named HTTP clients.
pub fn extract_candidates_from_source(source_file: &str, content: &str) -> Vec<EndpointCandidate> {
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
    let mut hits: Vec<(usize, EndpointCandidate)> = Vec::new();

    // We pre-compile each pattern once per call. The regex compilation cost
    // is small (~1 ms total), so caching across calls is not worth the
    // global state. If profiling shows otherwise, hoist these to a
    // `lazy_static` later.
    let fetch_re = Regex::new(patterns::FETCH).expect("FETCH regex valid");
    let axios_verb_re = Regex::new(patterns::AXIOS_VERB).expect("AXIOS_VERB regex valid");
    let http_client_verb_re =
        Regex::new(patterns::HTTP_CLIENT_VERB).expect("HTTP_CLIENT_VERB regex valid");
    let http_client_relative_verb_re = Regex::new(patterns::HTTP_CLIENT_RELATIVE_VERB)
        .expect("HTTP_CLIENT_RELATIVE_VERB regex valid");
    let axios_config_re = Regex::new(patterns::AXIOS_CONFIG).expect("AXIOS_CONFIG regex valid");
    let jquery_re = Regex::new(patterns::JQUERY_AJAX).expect("JQUERY_AJAX regex valid");
    let new_request_re = Regex::new(patterns::NEW_REQUEST).expect("NEW_REQUEST regex valid");
    let xhr_open_re = Regex::new(patterns::XHR_OPEN).expect("XHR_OPEN regex valid");
    let graphql_url_re =
        Regex::new(patterns::GRAPHQL_URL_CALL).expect("GRAPHQL_URL_CALL regex valid");
    let graphql_client_re =
        Regex::new(patterns::GRAPHQL_CLIENT_CALL).expect("GRAPHQL_CLIENT_CALL regex valid");
    let websocket_re = Regex::new(patterns::WEBSOCKET).expect("WEBSOCKET regex valid");
    let event_source_re = Regex::new(patterns::EVENT_SOURCE).expect("EVENT_SOURCE regex valid");
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
        if let (Some(matched), Some(endpoint)) = (
            cap.get(0),
            patterns::endpoint_from_fetch_concat(&cap, scrubbed_str, source_file),
        ) {
            hits.push((
                off,
                endpoint_candidate(endpoint, content, matched, "fetch", None),
            ));
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
        if let (Some(matched), Some(endpoint)) = (
            cap.get(0),
            patterns::endpoint_from_fetch_template(&cap, scrubbed_str, source_file),
        ) {
            hits.push((
                off,
                endpoint_candidate(endpoint, content, matched, "fetch", None),
            ));
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
        if let (Some(matched), Some(endpoint)) = (
            cap.get(0),
            patterns::endpoint_from_fetch(&cap, scrubbed_str, source_file),
        ) {
            hits.push((
                off,
                endpoint_candidate(endpoint, content, matched, "fetch", None),
            ));
        }
    }
    for cap in axios_verb_re.captures_iter(scrubbed_str) {
        let off = cap.get(0).map(|m| m.start()).unwrap_or(0);
        let verb = cap.get(1).map(|value| value.as_str().to_ascii_lowercase());
        if let (Some(matched), Some(verb), Some(endpoint)) = (
            cap.get(0),
            verb,
            patterns::endpoint_from_axios_verb(&cap, scrubbed_str, source_file),
        ) {
            hits.push((
                off,
                endpoint_candidate(
                    endpoint,
                    content,
                    matched,
                    &format!("axios.{verb}"),
                    Some("axios"),
                ),
            ));
        }
    }
    for cap in http_client_verb_re.captures_iter(scrubbed_str) {
        let receiver = cap.get(1).map(|value| value.as_str().to_string());
        let verb = cap.get(2).map(|value| value.as_str().to_ascii_lowercase());
        if let (Some(matched), Some(receiver), Some(verb), Some(endpoint)) = (
            cap.get(0),
            receiver,
            verb,
            patterns::endpoint_from_http_client_verb(&cap, scrubbed_str, source_file),
        ) {
            let candidate =
                http_client_endpoint_candidate(endpoint, content, matched, &receiver, &verb);
            hits.push((candidate.call.span.start_byte, candidate));
        }
    }
    for cap in http_client_relative_verb_re.captures_iter(scrubbed_str) {
        let Some(path) = cap.get(3).map(|value| value.as_str()) else {
            continue;
        };
        if path.starts_with('/') || path.contains(':') {
            continue;
        }
        let receiver = cap.get(1).map(|value| value.as_str().to_string());
        let verb = cap.get(2).map(|value| value.as_str().to_ascii_lowercase());
        if let (Some(matched), Some(receiver), Some(verb), Some(mut endpoint)) = (
            cap.get(0),
            receiver,
            verb,
            patterns::endpoint_from_http_client_verb(&cap, scrubbed_str, source_file),
        ) {
            endpoint.confidence = 0.65;
            let candidate =
                http_client_endpoint_candidate(endpoint, content, matched, &receiver, &verb);
            hits.push((candidate.call.span.start_byte, candidate));
        }
    }
    for cap in axios_config_re.captures_iter(scrubbed_str) {
        let off = cap.get(0).map(|m| m.start()).unwrap_or(0);
        if let (Some(matched), Some(endpoint)) = (
            cap.get(0),
            patterns::endpoint_from_axios_config(&cap, scrubbed_str, source_file),
        ) {
            hits.push((
                off,
                endpoint_candidate(endpoint, content, matched, "axios", Some("axios")),
            ));
        }
    }
    for cap in jquery_re.captures_iter(scrubbed_str) {
        let off = cap.get(0).map(|m| m.start()).unwrap_or(0);
        if let (Some(matched), Some(endpoint)) = (
            cap.get(0),
            patterns::endpoint_from_jquery(&cap, scrubbed_str, source_file),
        ) {
            let callee = if matched.as_str().trim_start().starts_with('$') {
                "$.ajax"
            } else {
                "jQuery.ajax"
            };
            hits.push((
                off,
                endpoint_candidate(endpoint, content, matched, callee, None),
            ));
        }
    }
    for cap in new_request_re.captures_iter(scrubbed_str) {
        let off = cap.get(0).map(|m| m.start()).unwrap_or(0);
        if let (Some(matched), Some(endpoint)) = (
            cap.get(0),
            patterns::endpoint_from_new_request(&cap, scrubbed_str, source_file),
        ) {
            hits.push((
                off,
                endpoint_candidate(endpoint, content, matched, "Request", None),
            ));
        }
    }
    for cap in xhr_open_re.captures_iter(scrubbed_str) {
        let receiver = cap.get(1).map(|value| value.as_str().to_string());
        if let (Some(matched), Some(receiver), Some(endpoint)) = (
            cap.get(0),
            receiver,
            patterns::endpoint_from_xhr_open(&cap, scrubbed_str, source_file),
        ) {
            hits.push((
                matched.start(),
                endpoint_candidate(
                    endpoint,
                    content,
                    matched,
                    &format!("{receiver}.open"),
                    Some(&receiver),
                ),
            ));
        }
    }
    for cap in graphql_url_re.captures_iter(scrubbed_str) {
        let callee = cap.get(1).map(|value| value.as_str().to_string());
        if let (Some(matched), Some(callee), Some(endpoint)) = (
            cap.get(0),
            callee,
            patterns::endpoint_from_graphql_url_call(&cap, scrubbed_str, source_file),
        ) {
            hits.push((
                matched.start(),
                endpoint_candidate(endpoint, content, matched, &callee, None),
            ));
        }
    }
    for cap in graphql_client_re.captures_iter(scrubbed_str) {
        let receiver = cap.get(1).map(|value| value.as_str().to_string());
        let verb = cap.get(2).map(|value| value.as_str().to_string());
        if let (Some(matched), Some(receiver), Some(verb), Some(endpoint)) = (
            cap.get(0),
            receiver,
            verb,
            patterns::endpoint_from_graphql_client_call(&cap, scrubbed_str, source_file),
        ) {
            hits.push((
                matched.start(),
                endpoint_candidate(
                    endpoint,
                    content,
                    matched,
                    &format!("{receiver}.{verb}"),
                    Some(&receiver),
                ),
            ));
        }
    }
    for cap in websocket_re.captures_iter(scrubbed_str) {
        if let (Some(matched), Some(endpoint)) = (
            cap.get(0),
            patterns::endpoint_from_websocket(&cap, scrubbed_str, source_file),
        ) {
            hits.push((
                matched.start(),
                endpoint_candidate(endpoint, content, matched, "WebSocket", None),
            ));
        }
    }
    for cap in event_source_re.captures_iter(scrubbed_str) {
        if let (Some(matched), Some(endpoint)) = (
            cap.get(0),
            patterns::endpoint_from_event_source(&cap, scrubbed_str, source_file),
        ) {
            hits.push((
                matched.start(),
                endpoint_candidate(endpoint, content, matched, "EventSource", None),
            ));
        }
    }

    // P2 final filter: drop any hit whose byte offset is NOT inside a
    // tree-sitter-confirmed call_expression / new_expression node.
    // When `ast_ranges == None` (parser bailed) we keep everything —
    // graceful degradation.
    let candidates: Vec<EndpointCandidate> = if let Some(ref ranges) = ast_ranges {
        hits.into_iter()
            .filter_map(|(off, mut candidate)| {
                let (start, end) = ranges.range_containing(off)?;
                bind_exact_callsite_facts(&mut candidate, source_file, content, start, end);
                Some(candidate)
            })
            .collect()
    } else {
        hits.into_iter()
            .map(|(_, mut candidate)| {
                assign_candidate_id(&mut candidate, source_file);
                candidate
            })
            .collect()
    };

    candidates
}

fn expanded_member_chain_receiver(
    source: &str,
    receiver_start: usize,
    receiver_leaf: &str,
) -> (String, usize) {
    let bytes = source.as_bytes();
    let mut chain_start = receiver_start;
    loop {
        let mut cursor = chain_start;
        while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
            cursor -= 1;
        }
        if cursor == 0 || bytes[cursor - 1] != b'.' {
            break;
        }
        cursor -= 1;
        while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
            cursor -= 1;
        }
        if cursor > 0 && bytes[cursor - 1] == b'?' {
            cursor -= 1;
            while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
                cursor -= 1;
            }
        }
        let identifier_end = cursor;
        while cursor > 0
            && (bytes[cursor - 1].is_ascii_alphanumeric()
                || matches!(bytes[cursor - 1], b'_' | b'$'))
        {
            cursor -= 1;
        }
        if cursor == identifier_end {
            break;
        }
        chain_start = cursor;
    }

    let receiver_end = receiver_start.saturating_add(receiver_leaf.len());
    let receiver = source[chain_start..receiver_end]
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    (receiver, chain_start)
}

fn legacy_candidate_visible(candidate: &EndpointCandidate) -> bool {
    matches!(
        candidate.endpoint.kind,
        CallSiteKind::Fetch
            | CallSiteKind::AxiosVerb
            | CallSiteKind::AxiosConfig
            | CallSiteKind::HttpClientVerb
            | CallSiteKind::JqueryAjax
            | CallSiteKind::NewRequest
            | CallSiteKind::HaeRoute
    ) && (candidate.endpoint.kind != CallSiteKind::HttpClientVerb
        || candidate.endpoint.path.starts_with('/'))
        && candidate
            .call
            .receiver
            .as_deref()
            .is_none_or(|receiver| !receiver.contains('.'))
}

fn http_client_endpoint_candidate(
    endpoint: Endpoint,
    source: &str,
    matched: regex::Match<'_>,
    receiver_leaf: &str,
    verb: &str,
) -> EndpointCandidate {
    let (receiver, chain_start) =
        expanded_member_chain_receiver(source, matched.start(), receiver_leaf);
    let mut candidate = endpoint_candidate(
        endpoint,
        source,
        matched,
        &format!("{receiver}.{verb}"),
        Some(&receiver),
    );
    if chain_start != candidate.call.span.start_byte {
        let line_start = source[..chain_start]
            .rfind('\n')
            .map(|offset| offset + 1)
            .unwrap_or(0);
        candidate.call.span.start_byte = chain_start;
        candidate.call.span.line = 1 + source[..chain_start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        candidate.call.span.column = chain_start.saturating_sub(line_start);
    }
    candidate
}

fn endpoint_candidate(
    endpoint: Endpoint,
    source: &str,
    matched: regex::Match<'_>,
    callee: &str,
    receiver: Option<&str>,
) -> EndpointCandidate {
    let start_byte = matched.start();
    let end_byte = matched.end();
    let line_start = source[..start_byte]
        .rfind('\n')
        .map(|offset| offset + 1)
        .unwrap_or(0);
    EndpointCandidate {
        candidate_id: String::new(),
        endpoint,
        call: CallSiteContext {
            callee: callee.to_string(),
            receiver: receiver.map(ToOwned::to_owned),
            span: SourceSpan {
                start_byte,
                end_byte,
                line: 1 + source[..start_byte]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count(),
                column: start_byte.saturating_sub(line_start),
            },
        },
        adapter: CallAdapter::Raw,
        arguments: Vec::new(),
        config: Vec::new(),
        parameters: Vec::new(),
        graphql_operation: None,
    }
}

fn bind_exact_callsite_facts(
    candidate: &mut EndpointCandidate,
    source_file: &str,
    source: &str,
    start_byte: usize,
    end_byte: usize,
) {
    let line_start = source[..start_byte]
        .rfind('\n')
        .map(|offset| offset + 1)
        .unwrap_or(0);
    candidate.call.span = SourceSpan {
        start_byte,
        end_byte,
        line: 1 + source[..start_byte]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count(),
        column: start_byte.saturating_sub(line_start),
    };

    let call_source = &source[start_byte..end_byte];
    let facts = patterns::facts_from_call(
        candidate.endpoint.kind,
        &candidate.endpoint.method,
        &candidate.endpoint.path,
        call_source,
    );
    candidate.adapter = facts.adapter;
    candidate.arguments = facts.arguments;
    candidate.config = facts.config;
    candidate.parameters = facts.parameters;
    candidate.graphql_operation = facts.graphql_operation;
    assign_candidate_id(candidate, source_file);
}

fn assign_candidate_id(candidate: &mut EndpointCandidate, source_file: &str) {
    let mut digest = Sha256::new();
    digest.update(b"golish-js-callsite-v1\0");
    digest.update(source_file.as_bytes());
    digest.update(b"\0");
    digest.update(candidate.call.span.start_byte.to_le_bytes());
    digest.update(candidate.call.span.end_byte.to_le_bytes());
    digest.update(b"\0");
    digest.update(candidate.call.callee.as_bytes());

    let mut id = String::from("js-callsite-v1:");
    for byte in digest.finalize() {
        write!(&mut id, "{byte:02x}").expect("writing to String cannot fail");
    }
    candidate.candidate_id = id;
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
        let mut found = extract_from_source(path_ref, source.as_ref());
        if found.is_empty() {
            report.skipped.push(SkippedFile {
                file: path_ref.to_string(),
                reason: "no recognized HTTP call patterns".to_string(),
            });
            continue;
        }
        for endpoint in &found {
            if seen.insert((endpoint.method.clone(), endpoint.path.clone())) {
                report.unique += 1;
            }
        }
        report.endpoints.append(&mut found);
    }
    report
}

/// Extract from many files without collapsing source-local call-site facts.
pub fn extract_candidates_from_files<I, S1, S2>(files: I) -> CandidateExtractReport
where
    I: IntoIterator<Item = (S1, S2)>,
    S1: AsRef<str>,
    S2: AsRef<str>,
{
    let mut report = CandidateExtractReport::default();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for (path, source) in files {
        let path_ref = path.as_ref();
        let source_ref = source.as_ref();
        let mut found = extract_candidates_from_source(path_ref, source_ref);

        if found.is_empty() {
            report.skipped.push(SkippedFile {
                file: path_ref.to_string(),
                reason: "no recognized HTTP call patterns".to_string(),
            });
            continue;
        }

        for candidate in &found {
            if seen.insert((
                candidate.endpoint.method.clone(),
                candidate.endpoint.path.clone(),
            )) {
                report.unique += 1;
            }
        }
        report.candidates.append(&mut found);
    }

    report
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
