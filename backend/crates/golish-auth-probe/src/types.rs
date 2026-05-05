//! Public types — one place for the probe's input / output schema.

use serde::{Deserialize, Serialize};

/// Which test we ran against an endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    /// Hit endpoint without any auth headers.
    Anonymous,
    /// Hit with token A but with B's resource ID.
    CrossUser,
    /// Hit with low-priv token against admin-shaped path.
    Privilege,
}

/// How a token is sourced for a probe round.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TokenSource {
    /// Look up a credential by name in `credential_vault`.
    /// The actual lookup is done outside this crate (the Tool wrapper
    /// does the SQL); we just receive the resolved string.
    Plain { value: String },
    /// No token — anonymous request.
    None,
}

/// One round of a probe — what we sent and what came back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Round {
    /// HTTP status code, or `0` if the request never completed (timeout / DNS).
    pub status: u16,
    /// Length of the response body in bytes.
    pub body_len: usize,
    /// First 200 bytes of the response body, UTF-8 lossy. Used for
    /// human-readable diff in the finding evidence.
    pub snippet: String,
    /// Outcome category for the comparison engine.
    pub outcome: RoundOutcome,
    /// `Retry-After` value if the server was rate-limiting (header in seconds).
    #[serde(default)]
    pub retry_after_secs: Option<u32>,
}

/// High-level classification of one round's response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundOutcome {
    /// 2xx — endpoint served the resource.
    Success,
    /// 401 / 403 — auth was required and denied.
    AuthDenied,
    /// 404 — resource not found (often legitimate for cross-user probes).
    NotFound,
    /// 429 — rate-limited. Counted into `summary.rate_limited_count`.
    RateLimited,
    /// 5xx — server error.
    ServerError,
    /// Anything else (3xx redirects, 4xx that isn't 401/403/404, etc.).
    Other,
    /// Request never completed (DNS, TLS, timeout). Counted into
    /// `summary.network_error_count`.
    NetworkError,
}

/// Comparison result for a (endpoint, scenario) tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Confirmed vulnerability.
    Vulnerable,
    /// Pattern is suspicious but inconclusive — manual review recommended.
    Potential,
    /// Endpoint correctly denied / behaved as expected.
    NotVulnerable,
    /// Probe couldn't decide (rate-limited, server errors, etc.).
    Inconclusive,
    /// Network error prevented any judgement.
    Error,
}

/// Severity assigned to a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

/// Bundled rounds + diff summary that backs a finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub round_1: Option<Round>,
    pub round_2: Option<Round>,
    pub round_3: Option<Round>,
    /// Short, human-readable diff explanation (e.g. "anon and authed
    /// return identical body").
    pub diff_summary: String,
}

/// One finding for a single (endpoint, scenario) combo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub endpoint: golish_js_analyzer::Endpoint,
    pub scenario: Scenario,
    pub verdict: Verdict,
    pub severity: Severity,
    pub evidence: Evidence,
}

/// Tunables for a probe run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeConfig {
    /// Base URL the endpoints were extracted from (e.g. `https://target.example.com`).
    pub base_url: String,
    /// Resolved tokens for the two test users. `b` is optional —
    /// cross-user scenarios are skipped if absent.
    pub token_a: TokenSource,
    pub token_b: TokenSource,
    /// IDs known to belong to each user, by index. Used for cross-user
    /// substitution. Skipped if either pool is empty.
    pub id_pool_a: Vec<String>,
    pub id_pool_b: Vec<String>,
    /// Which scenarios to run. Default: all three.
    pub scenarios: Vec<Scenario>,
    /// Min ms between requests to the same host. Default 1000.
    pub rate_limit_ms: u64,
    /// Per-request timeout in ms. Default 10000.
    pub timeout_ms: u64,
    /// Cap on how many endpoints to probe per run. Default 500.
    pub max_endpoints: usize,
    /// Custom user-agent. Defaults to a browser-like UA in [`probe`].
    pub user_agent: Option<String>,
    /// When `false`, only safe verbs (GET / HEAD / OPTIONS) are probed.
    /// When `true`, the caller has explicitly accepted the risk of
    /// probing mutating verbs. Default: `false`.
    pub include_mutating: bool,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            token_a: TokenSource::None,
            token_b: TokenSource::None,
            id_pool_a: Vec::new(),
            id_pool_b: Vec::new(),
            scenarios: vec![Scenario::Anonymous, Scenario::CrossUser, Scenario::Privilege],
            rate_limit_ms: 1000,
            timeout_ms: 10_000,
            max_endpoints: 500,
            user_agent: None,
            include_mutating: false,
        }
    }
}

/// Aggregated counts across all findings — surfaced as the `summary` block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProbeSummary {
    pub by_severity: std::collections::BTreeMap<String, usize>,
    pub by_scenario: std::collections::BTreeMap<String, usize>,
    pub total_requests: usize,
    pub rate_limited_count: usize,
    pub network_error_count: usize,
}

/// Result of a probe run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProbeReport {
    pub tested_count: usize,
    pub skipped_count: usize,
    pub findings: Vec<Finding>,
    pub summary: ProbeSummary,
}
