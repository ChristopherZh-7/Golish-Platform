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

use crate::{
    ArgumentFact, ArgumentRole, AuthHint, CallAdapter, ConfigFact, Endpoint, EndpointSource,
    GraphqlOperationFact, GraphqlOperationKind, ParameterFact, ParameterLocation,
    ParameterValueType, UrlKind,
};

/// Family of HTTP-call call-site this endpoint was extracted from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallSiteKind {
    Fetch,
    AxiosVerb,
    AxiosConfig,
    HttpClientVerb,
    JqueryAjax,
    NewRequest,
    XmlHttpRequest,
    Graphql,
    WebSocket,
    EventSource,
    HaeRoute,
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

/// `client.<verb>('/path', ...)` — custom axios/request wrapper helpers.
///
/// Captures client name, verb, and URL. This intentionally requires a
/// root-relative URL to avoid noisy matches such as date/string helpers.
pub(crate) const HTTP_CLIENT_VERB: &str = r#"(?m)\b([A-Za-z_$][A-Za-z0-9_$]*)\s*\.\s*(get|post|put|patch|delete|head|options|download|upload)\s*\(\s*[`'"](/[^`'"]+)[`'"]"#;

/// Candidate-only companion for relative custom-client paths such as
/// `admin.get('users')`. The legacy endpoint API filters these back out;
/// contextual consumers may retain them only when `admin` has a proven base.
pub(crate) const HTTP_CLIENT_RELATIVE_VERB: &str = r#"(?m)\b([A-Za-z_$][A-Za-z0-9_$]*)\s*\.\s*(get|post|put|patch|delete|head|options|download|upload)\s*\(\s*[`'"]([^`'"]+)[`'"]"#;

/// `axios({ url: '/path', method: 'POST' })` — captures url; method parsed by helper.
pub(crate) const AXIOS_CONFIG: &str =
    r#"(?m)\baxios\s*\(\s*\{[^}]*?\burl\s*:\s*[`'"]([^`'"]+)[`'"]"#;

/// `$.ajax({ url: '/path', type: 'POST' })` or `jQuery.ajax(...)`.
pub(crate) const JQUERY_AJAX: &str =
    r#"(?m)(?:\$|jQuery)\s*\.\s*ajax\s*\(\s*\{[^}]*?\burl\s*:\s*[`'"]([^`'"]+)[`'"]"#;

/// `new Request('/path', { method: 'PUT' })`.
pub(crate) const NEW_REQUEST: &str = r#"(?m)\bnew\s+Request\s*\(\s*[`'"]([^`'"]+)[`'"]"#;

/// `xhr.open('POST', '/path', ...)` — receiver, method and URL.
pub(crate) const XHR_OPEN: &str = r#"(?m)\b([A-Za-z_$][A-Za-z0-9_$]*)\s*\.\s*open\s*\(\s*[`'"](GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)[`'"]\s*,\s*[`'"]([^`'"]+)[`'"]"#;

/// URL-first GraphQL helpers. Opaque wrappers outside this closed name set are
/// intentionally left to the raw custom-client path.
pub(crate) const GRAPHQL_URL_CALL: &str =
    r#"(?m)\b(graphql|graphqlRequest|gqlRequest)\s*\(\s*[`'"]([^`'"]+)[`'"]"#;

/// Apollo/urql-style client calls. They carry no URL at this layer; the empty
/// raw path is an explicit unresolved input for the contextual resolver.
pub(crate) const GRAPHQL_CLIENT_CALL: &str =
    r#"(?m)\b([A-Za-z_$][A-Za-z0-9_$]*)\s*\.\s*(query|mutate|subscribe)\s*\(\s*\{"#;

pub(crate) const WEBSOCKET: &str = r#"(?m)\bnew\s+WebSocket\s*\(\s*[`'"]([^`'"]+)[`'"]"#;

pub(crate) const EVENT_SOURCE: &str = r#"(?m)\bnew\s+EventSource\s*\(\s*[`'"]([^`'"]+)[`'"]"#;

/// `fetch('/api/users/' + id, ...)` — captures the literal prefix when
/// the URL is built by string concatenation. The `+` after the closing
/// quote is the disambiguator that prevents this from re-matching plain
/// `FETCH` literal call sites.
pub(crate) const FETCH_CONCAT: &str = r#"(?m)\bfetch\s*\(\s*[`'"]([^`'"]+)[`'"]\s*\+"#;

/// `` fetch(`/api/users/${id}`, ...) `` — backtick-quoted template literal
/// with at least one `${...}` placeholder. We capture the raw template
/// body (placeholders preserved) so callers see the same text the model
/// would.
pub(crate) const FETCH_TEMPLATE: &str = r#"(?m)\bfetch\s*\(\s*`([^`]*\$\{[^`]*)`"#;

#[derive(Debug, Default)]
pub(crate) struct CandidateFacts {
    pub adapter: CallAdapter,
    pub arguments: Vec<ArgumentFact>,
    pub config: Vec<ConfigFact>,
    pub parameters: Vec<ParameterFact>,
    pub graphql_operation: Option<GraphqlOperationFact>,
}

/// Interpret only the exact AST-confirmed call expression supplied by the
/// caller. No surrounding source window is consulted, which prevents fields
/// from an adjacent minified call from leaking into this candidate.
pub(crate) fn facts_from_call(
    kind: CallSiteKind,
    method: &str,
    path: &str,
    call_source: &str,
) -> CandidateFacts {
    let arguments = call_arguments(call_source);
    let mut facts = CandidateFacts::default();

    match kind {
        CallSiteKind::Fetch => extract_fetch_facts(path, &arguments, &mut facts),
        CallSiteKind::AxiosVerb | CallSiteKind::AxiosConfig => {
            extract_axios_facts(kind, method, path, &arguments, &mut facts);
        }
        CallSiteKind::NewRequest => extract_request_facts(path, &arguments, &mut facts),
        CallSiteKind::JqueryAjax => extract_jquery_facts(path, &arguments, &mut facts),
        CallSiteKind::XmlHttpRequest => extract_xhr_facts(path, &arguments, &mut facts),
        CallSiteKind::Graphql => extract_graphql_facts(path, &arguments, &mut facts),
        CallSiteKind::WebSocket => {
            extract_stream_facts(CallAdapter::WebSocket, path, &arguments, &mut facts);
        }
        CallSiteKind::EventSource => {
            extract_stream_facts(CallAdapter::EventSource, path, &arguments, &mut facts);
        }
        CallSiteKind::HttpClientVerb | CallSiteKind::HaeRoute => {}
    }

    facts
}

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
        source: EndpointSource::Regex,
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
        source: EndpointSource::Regex,
    })
}

pub(crate) fn endpoint_from_http_client_verb(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    let client = cap.get(1)?.as_str();
    if client.eq_ignore_ascii_case("axios") {
        return None;
    }

    let verb = cap.get(2)?.as_str();
    let method = method_from_client_verb(verb);
    let path = cap.get(3)?.as_str().to_string();
    let match_start = cap.get(0)?.start();
    let (has_path_params, id_param_position) = analyze_path(&path);
    Some(Endpoint {
        method,
        path,
        auth: auth_from_window(window_after(source, match_start, 400)),
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.78,
        kind: CallSiteKind::HttpClientVerb,
        url_kind: UrlKind::Literal,
        has_path_params,
        id_param_position,
        source: EndpointSource::Regex,
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
        source: EndpointSource::Regex,
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
        source: EndpointSource::Regex,
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
        source: EndpointSource::Regex,
    })
}

pub(crate) fn endpoint_from_xhr_open(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    let method = cap.get(2)?.as_str().to_uppercase();
    let path = cap.get(3)?.as_str().to_string();
    let match_start = cap.get(0)?.start();
    let (has_path_params, id_param_position) = analyze_path(&path);
    Some(Endpoint {
        method,
        path,
        auth: AuthHint::None,
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.95,
        kind: CallSiteKind::XmlHttpRequest,
        url_kind: UrlKind::Literal,
        has_path_params,
        id_param_position,
        source: EndpointSource::Regex,
    })
}

pub(crate) fn endpoint_from_graphql_url_call(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    let path = cap.get(2)?.as_str().to_string();
    let match_start = cap.get(0)?.start();
    let (has_path_params, id_param_position) = analyze_path(&path);
    Some(Endpoint {
        method: "POST".to_string(),
        path,
        auth: AuthHint::None,
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.9,
        kind: CallSiteKind::Graphql,
        url_kind: UrlKind::Literal,
        has_path_params,
        id_param_position,
        source: EndpointSource::Regex,
    })
}

pub(crate) fn endpoint_from_graphql_client_call(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    let match_start = cap.get(0)?.start();
    Some(Endpoint {
        method: "POST".to_string(),
        path: String::new(),
        auth: AuthHint::None,
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.55,
        kind: CallSiteKind::Graphql,
        url_kind: UrlKind::Literal,
        has_path_params: false,
        id_param_position: None,
        source: EndpointSource::Regex,
    })
}

pub(crate) fn endpoint_from_websocket(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    endpoint_from_stream_constructor(cap, source, source_file, CallSiteKind::WebSocket)
}

pub(crate) fn endpoint_from_event_source(
    cap: &Captures,
    source: &str,
    source_file: &str,
) -> Option<Endpoint> {
    endpoint_from_stream_constructor(cap, source, source_file, CallSiteKind::EventSource)
}

fn endpoint_from_stream_constructor(
    cap: &Captures,
    source: &str,
    source_file: &str,
    kind: CallSiteKind,
) -> Option<Endpoint> {
    let path = cap.get(1)?.as_str().to_string();
    let match_start = cap.get(0)?.start();
    let (has_path_params, id_param_position) = analyze_path(&path);
    Some(Endpoint {
        method: "GET".to_string(),
        path,
        auth: AuthHint::None,
        source_file: source_file.to_string(),
        line: line_of(source, match_start),
        confidence: 0.95,
        kind,
        url_kind: UrlKind::Literal,
        has_path_params,
        id_param_position,
        source: EndpointSource::Regex,
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
        source: EndpointSource::Regex,
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
    let trimmed = template_body.trim_start_matches('/').trim_end_matches('/');
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
        source: EndpointSource::Regex,
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

fn method_from_client_verb(verb: &str) -> String {
    match verb.to_ascii_lowercase().as_str() {
        "download" => "GET".to_string(),
        "upload" => "POST".to_string(),
        other => other.to_ascii_uppercase(),
    }
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
        || (lower.contains("withcredentials") && (lower.contains("true") || lower.contains("!0")))
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

fn extract_fetch_facts(path: &str, arguments: &[&str], facts: &mut CandidateFacts) {
    facts.adapter = CallAdapter::Fetch;
    add_query_parameters(path, facts);
    add_path_parameters(path, facts);
    if let Some(url) = arguments.first() {
        add_argument(0, ArgumentRole::Url, url, facts);
    }
    if let Some(config) = arguments.get(1) {
        add_argument(1, ArgumentRole::Config, config, facts);
        add_config_object(config, facts);
        add_named_config_parameters(config, "headers", ParameterLocation::Header, facts);
        if let Some(body) = object_field(config, "body") {
            let location = if expression_is_call_to(body, &["URLSearchParams", "FormData"]) {
                ParameterLocation::Form
            } else {
                ParameterLocation::Body
            };
            add_object_parameters(body, location, facts);
        }
    }
}

fn extract_request_facts(path: &str, arguments: &[&str], facts: &mut CandidateFacts) {
    extract_fetch_facts(path, arguments, facts);
    facts.adapter = CallAdapter::Request;
}

fn extract_axios_facts(
    kind: CallSiteKind,
    method: &str,
    path: &str,
    arguments: &[&str],
    facts: &mut CandidateFacts,
) {
    facts.adapter = CallAdapter::Axios;
    add_query_parameters(path, facts);
    add_path_parameters(path, facts);

    if kind == CallSiteKind::AxiosConfig {
        if let Some(config) = arguments.first() {
            add_argument(0, ArgumentRole::Config, config, facts);
            add_axios_config(config, facts);
        }
        return;
    }

    if let Some(url) = arguments.first() {
        add_argument(0, ArgumentRole::Url, url, facts);
    }
    let method_has_body = matches!(method, "POST" | "PUT" | "PATCH");
    if method_has_body {
        if let Some(body) = arguments.get(1) {
            add_argument(1, ArgumentRole::Body, body, facts);
            add_object_parameters(body, ParameterLocation::Body, facts);
        }
        if let Some(config) = arguments.get(2) {
            add_argument(2, ArgumentRole::Config, config, facts);
            add_axios_config(config, facts);
        }
    } else if let Some(config) = arguments.get(1) {
        add_argument(1, ArgumentRole::Config, config, facts);
        add_axios_config(config, facts);
    }
}

fn add_axios_config(config: &str, facts: &mut CandidateFacts) {
    add_config_object(config, facts);
    add_named_config_parameters(config, "params", ParameterLocation::Query, facts);
    add_named_config_parameters(config, "data", ParameterLocation::Body, facts);
    add_named_config_parameters(config, "headers", ParameterLocation::Header, facts);
}

fn extract_jquery_facts(path: &str, arguments: &[&str], facts: &mut CandidateFacts) {
    facts.adapter = CallAdapter::JQuery;
    add_query_parameters(path, facts);
    add_path_parameters(path, facts);
    if let Some(config) = arguments.first() {
        add_argument(0, ArgumentRole::Config, config, facts);
        add_config_object(config, facts);
        add_named_config_parameters(config, "data", ParameterLocation::Form, facts);
        add_named_config_parameters(config, "headers", ParameterLocation::Header, facts);
    }
}

fn extract_xhr_facts(path: &str, arguments: &[&str], facts: &mut CandidateFacts) {
    facts.adapter = CallAdapter::XmlHttpRequest;
    add_query_parameters(path, facts);
    add_path_parameters(path, facts);
    if let Some(method) = arguments.first() {
        add_argument(0, ArgumentRole::Unknown, method, facts);
    }
    if let Some(url) = arguments.get(1) {
        add_argument(1, ArgumentRole::Url, url, facts);
    }
}

fn extract_graphql_facts(path: &str, arguments: &[&str], facts: &mut CandidateFacts) {
    facts.adapter = CallAdapter::Graphql;
    add_query_parameters(path, facts);
    add_path_parameters(path, facts);

    let mut config_index = 0;
    if !path.is_empty() {
        if let Some(url) = arguments.first() {
            add_argument(0, ArgumentRole::Url, url, facts);
        }
        config_index = 1;
    }

    if let Some(config_or_document) = arguments.get(config_index) {
        if is_object_expression(config_or_document) {
            add_argument(
                config_index,
                ArgumentRole::Config,
                config_or_document,
                facts,
            );
            add_config_object(config_or_document, facts);
            if let Some(document) = object_field(config_or_document, "query") {
                add_graphql_document(document, facts);
            }
            if let Some(variables) = object_field(config_or_document, "variables") {
                add_object_parameters(variables, ParameterLocation::GraphqlVariable, facts);
            }
        } else {
            add_argument(
                config_index,
                ArgumentRole::GraphqlDocument,
                config_or_document,
                facts,
            );
            add_graphql_document(config_or_document, facts);
        }
    }

    if let Some(variables) = arguments.get(config_index + 1) {
        add_argument(
            config_index + 1,
            ArgumentRole::GraphqlVariables,
            variables,
            facts,
        );
        add_object_parameters(variables, ParameterLocation::GraphqlVariable, facts);
    }
}

fn extract_stream_facts(
    adapter: CallAdapter,
    path: &str,
    arguments: &[&str],
    facts: &mut CandidateFacts,
) {
    facts.adapter = adapter;
    add_query_parameters(path, facts);
    add_path_parameters(path, facts);
    if let Some(url) = arguments.first() {
        add_argument(0, ArgumentRole::Url, url, facts);
    }
}

fn add_argument(index: usize, role: ArgumentRole, expression: &str, facts: &mut CandidateFacts) {
    let value_type = expression_type(expression);
    facts.arguments.push(ArgumentFact {
        index,
        role,
        value_type,
        dynamic: value_type == ParameterValueType::Unknown,
    });
}

fn add_config_object(expression: &str, facts: &mut CandidateFacts) {
    for (name, value) in object_fields(expression) {
        if facts.config.iter().any(|fact| fact.name == name) {
            continue;
        }
        facts.config.push(ConfigFact {
            name,
            value_type: expression_type(value),
        });
    }
}

fn add_named_config_parameters(
    config: &str,
    field: &str,
    location: ParameterLocation,
    facts: &mut CandidateFacts,
) {
    if let Some(expression) = object_field(config, field) {
        add_object_parameters(expression, location, facts);
    }
}

fn add_object_parameters(
    expression: &str,
    location: ParameterLocation,
    facts: &mut CandidateFacts,
) {
    let object = unwrap_value_container(expression).unwrap_or(expression);
    for (name, value) in object_fields(object) {
        push_parameter(
            ParameterFact {
                name,
                location,
                value_type: expression_type(value),
            },
            facts,
        );
    }
}

fn add_query_parameters(path: &str, facts: &mut CandidateFacts) {
    let Some((_, query_and_fragment)) = path.split_once('?') else {
        return;
    };
    let query = query_and_fragment.split('#').next().unwrap_or_default();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let name = pair.split_once('=').map_or(pair, |(name, _)| name).trim();
        if !name.is_empty() {
            push_parameter(
                ParameterFact {
                    name: name.to_string(),
                    location: ParameterLocation::Query,
                    value_type: ParameterValueType::Unknown,
                },
                facts,
            );
        }
    }
}

fn add_path_parameters(path: &str, facts: &mut CandidateFacts) {
    for segment in path.split('?').next().unwrap_or(path).split('/') {
        let name = if let Some(name) = segment.strip_prefix(':') {
            Some(name)
        } else if segment.starts_with('{') && segment.ends_with('}') {
            segment.get(1..segment.len().saturating_sub(1))
        } else if segment.starts_with("${") && segment.ends_with('}') {
            segment.get(2..segment.len().saturating_sub(1))
        } else {
            None
        };
        if let Some(name) = name.filter(|name| is_identifier(name)) {
            push_parameter(
                ParameterFact {
                    name: name.to_string(),
                    location: ParameterLocation::Path,
                    value_type: ParameterValueType::Unknown,
                },
                facts,
            );
        }
    }
}

fn add_graphql_document(expression: &str, facts: &mut CandidateFacts) {
    let operation = regex::RegexBuilder::new(
        r"\b(query|mutation|subscription)(?:\s+([A-Za-z_][A-Za-z0-9_]*))?",
    )
    .case_insensitive(true)
    .build()
    .expect("GraphQL operation regex is valid");
    if let Some(captures) = operation.captures(expression) {
        let kind = match captures
            .get(1)
            .map(|value| value.as_str().to_ascii_lowercase())
            .as_deref()
        {
            Some("mutation") => GraphqlOperationKind::Mutation,
            Some("subscription") => GraphqlOperationKind::Subscription,
            _ => GraphqlOperationKind::Query,
        };
        facts.graphql_operation = Some(GraphqlOperationFact {
            kind,
            name: captures.get(2).map(|value| value.as_str().to_string()),
        });
    }

    let variables =
        regex::Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)").expect("GraphQL variable regex is valid");
    for captures in variables.captures_iter(expression) {
        if let Some(name) = captures.get(1) {
            push_parameter(
                ParameterFact {
                    name: name.as_str().to_string(),
                    location: ParameterLocation::GraphqlVariable,
                    value_type: ParameterValueType::Unknown,
                },
                facts,
            );
        }
    }
}

fn push_parameter(parameter: ParameterFact, facts: &mut CandidateFacts) {
    if let Some(existing) = facts
        .parameters
        .iter_mut()
        .find(|existing| existing.name == parameter.name && existing.location == parameter.location)
    {
        if existing.value_type == ParameterValueType::Unknown
            && parameter.value_type != ParameterValueType::Unknown
        {
            existing.value_type = parameter.value_type;
        }
        return;
    }
    facts.parameters.push(parameter);
}

fn call_arguments(call_source: &str) -> Vec<&str> {
    let Some(open) = find_unquoted(call_source, b'(') else {
        return Vec::new();
    };
    let Some(close) = matching_closing(call_source, open, b'(', b')') else {
        return Vec::new();
    };
    split_top_level(&call_source[open + 1..close], b',')
}

fn object_fields(expression: &str) -> Vec<(String, &str)> {
    let expression = expression.trim();
    let Some(body) = expression
        .strip_prefix('{')
        .and_then(|body| body.strip_suffix('}'))
    else {
        return Vec::new();
    };

    split_top_level(body, b',')
        .into_iter()
        .filter_map(|field| {
            let field = field.trim();
            if field.is_empty() || field.starts_with("...") {
                return None;
            }
            if let Some(colon) = find_top_level(field, b':') {
                let name = field_name(&field[..colon])?;
                let value = field[colon + 1..].trim();
                return (!value.is_empty()).then_some((name, value));
            }
            is_identifier(field).then_some((field.to_string(), field))
        })
        .collect()
}

fn object_field<'a>(expression: &'a str, wanted: &str) -> Option<&'a str> {
    object_fields(expression)
        .into_iter()
        .find_map(|(name, value)| (name == wanted).then_some(value))
}

fn field_name(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if is_identifier(raw) {
        return Some(raw.to_string());
    }
    unquote_literal(raw).map(ToOwned::to_owned)
}

fn unwrap_value_container(expression: &str) -> Option<&str> {
    const NAMES: &[&str] = &["JSON.stringify", "URLSearchParams", "FormData", "Headers"];
    if !expression_is_call_to(expression, NAMES) {
        return None;
    }
    call_arguments(expression).into_iter().next()
}

fn expression_is_call_to(expression: &str, names: &[&str]) -> bool {
    let expression = expression
        .trim()
        .strip_prefix("new ")
        .unwrap_or(expression.trim());
    let Some(open) = find_unquoted(expression, b'(') else {
        return false;
    };
    let callee = expression[..open]
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    names.iter().any(|name| callee == *name)
}

fn is_object_expression(expression: &str) -> bool {
    let expression = expression.trim();
    expression.starts_with('{') && expression.ends_with('}')
}

fn expression_type(expression: &str) -> ParameterValueType {
    let expression = expression.trim();
    if expression.is_empty() {
        return ParameterValueType::Unknown;
    }
    if expression.starts_with('{') && expression.ends_with('}') {
        return ParameterValueType::Object;
    }
    if expression.starts_with('[') && expression.ends_with(']') {
        return ParameterValueType::Array;
    }
    if expression == "true" || expression == "false" {
        return ParameterValueType::Boolean;
    }
    if expression == "null" {
        return ParameterValueType::Null;
    }
    if expression.parse::<f64>().is_ok() {
        return ParameterValueType::Number;
    }
    if let Some(literal) = unquote_literal(expression) {
        return if expression.starts_with('`') && literal.contains("${") {
            ParameterValueType::Unknown
        } else {
            ParameterValueType::String
        };
    }
    ParameterValueType::Unknown
}

fn unquote_literal(expression: &str) -> Option<&str> {
    let bytes = expression.as_bytes();
    if bytes.len() < 2 || !matches!(bytes[0], b'\'' | b'"' | b'`') {
        return None;
    }
    (bytes.last() == Some(&bytes[0])).then_some(&expression[1..expression.len() - 1])
}

fn is_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    matches!(first, b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'$')
        && bytes.all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'$'))
}

fn split_top_level(input: &str, delimiter: u8) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut paren = 0usize;
    let mut brace = 0usize;
    let mut bracket = 0usize;
    let mut quote: Option<u8> = None;
    let mut escaped = false;

    for (index, byte) in input.bytes().enumerate() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == active_quote {
                quote = None;
            }
            continue;
        }
        match byte {
            b'\'' | b'"' | b'`' => quote = Some(byte),
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'{' => brace += 1,
            b'}' => brace = brace.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            _ if byte == delimiter && paren == 0 && brace == 0 && bracket == 0 => {
                parts.push(input[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(input[start..].trim());
    parts
}

fn find_top_level(input: &str, needle: u8) -> Option<usize> {
    let mut paren = 0usize;
    let mut brace = 0usize;
    let mut bracket = 0usize;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    for (index, byte) in input.bytes().enumerate() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == active_quote {
                quote = None;
            }
            continue;
        }
        match byte {
            b'\'' | b'"' | b'`' => quote = Some(byte),
            b'(' => paren += 1,
            b')' => paren = paren.saturating_sub(1),
            b'{' => brace += 1,
            b'}' => brace = brace.saturating_sub(1),
            b'[' => bracket += 1,
            b']' => bracket = bracket.saturating_sub(1),
            _ if byte == needle && paren == 0 && brace == 0 && bracket == 0 => {
                return Some(index);
            }
            _ => {}
        }
    }
    None
}

fn find_unquoted(input: &str, needle: u8) -> Option<usize> {
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    for (index, byte) in input.bytes().enumerate() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == active_quote {
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
        } else if byte == needle {
            return Some(index);
        }
    }
    None
}

fn matching_closing(input: &str, open: usize, opening: u8, closing: u8) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    for (index, byte) in input.bytes().enumerate().skip(open) {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
        } else if byte == opening {
            depth += 1;
        } else if byte == closing {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}
