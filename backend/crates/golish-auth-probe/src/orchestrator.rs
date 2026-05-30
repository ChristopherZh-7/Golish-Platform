//! Top-level orchestrator — `probe()` is the single entry-point for
//! the Tool wrapper.

use std::time::Duration;

use anyhow::Result;
use golish_js_analyzer::Endpoint;
use tokio::time::sleep;

use crate::compare::compare_rounds;
use crate::request::{build_client, execute_round};
use crate::substitute::{substitute_id, SubstituteKind};
use crate::types::{
    Evidence, ProbeConfig, ProbeFinding, ProbeReport, ProbeSummary, Round, RoundOutcome, Scenario,
    Severity, TokenSource, Verdict,
};

/// Run the probe over a list of endpoints.
///
/// Returns a [`ProbeReport`] with one [`ProbeFinding`] per (endpoint, scenario)
/// tuple that yielded a verdict.
pub async fn probe(endpoints: &[Endpoint], cfg: &ProbeConfig) -> Result<ProbeReport> {
    let client = build_client(cfg.timeout_ms, cfg.user_agent.as_deref())?;
    let mut report = ProbeReport::default();

    let total_targets = endpoints.len().min(cfg.max_endpoints);
    for ep in endpoints.iter().take(total_targets) {
        if !is_probe_safe(ep, cfg.include_mutating) {
            report.skipped_count += 1;
            continue;
        }
        report.tested_count += 1;

        for scenario in &cfg.scenarios {
            let outcome = run_scenario(&client, ep, *scenario, cfg, &mut report.summary).await;
            if let Some((rounds, verdict, severity, diff)) = outcome {
                let evidence = Evidence {
                    round_1: rounds.first().cloned(),
                    round_2: rounds.get(1).cloned(),
                    round_3: rounds.get(2).cloned(),
                    diff_summary: diff,
                };
                let scen_key = scenario_key(*scenario);
                let sev_key = severity_key(severity);
                *report.summary.by_scenario.entry(scen_key).or_insert(0) += 1;
                *report.summary.by_severity.entry(sev_key).or_insert(0) += 1;
                report.findings.push(ProbeFinding {
                    endpoint: ep.clone(),
                    scenario: *scenario,
                    verdict,
                    severity,
                    evidence,
                });
            }
        }

        if cfg.rate_limit_ms > 0 {
            sleep(Duration::from_millis(cfg.rate_limit_ms)).await;
        }
    }

    Ok(report)
}

/// Return `true` if it's safe to probe this endpoint with the current
/// `include_mutating` policy.
fn is_probe_safe(endpoint: &Endpoint, include_mutating: bool) -> bool {
    if include_mutating {
        return true;
    }
    matches!(endpoint.method.as_str(), "GET" | "HEAD" | "OPTIONS")
}

async fn run_scenario(
    client: &reqwest::Client,
    ep: &Endpoint,
    scenario: Scenario,
    cfg: &ProbeConfig,
    summary: &mut ProbeSummary,
) -> Option<(Vec<Round>, Verdict, Severity, String)> {
    match scenario {
        Scenario::Anonymous => run_anonymous(client, ep, cfg, summary).await,
        Scenario::CrossUser => run_cross_user(client, ep, cfg, summary).await,
        Scenario::Privilege => run_privilege(client, ep, cfg, summary).await,
    }
}

async fn run_anonymous(
    client: &reqwest::Client,
    ep: &Endpoint,
    cfg: &ProbeConfig,
    summary: &mut ProbeSummary,
) -> Option<(Vec<Round>, Verdict, Severity, String)> {
    let path = substitute_id(ep, SubstituteKind::SameId { default_id: "1" })?;
    let url = join_url(&cfg.base_url, &path);

    let r1 = execute_round(client, &ep.method, &url, &TokenSource::None).await;
    summary.total_requests += 1;
    bump_summary_for_round(&r1, summary);

    let r2 = match &cfg.token_a {
        TokenSource::None => None,
        token => {
            let r = execute_round(client, &ep.method, &url, token).await;
            summary.total_requests += 1;
            bump_summary_for_round(&r, summary);
            Some(r)
        }
    };

    let rounds: Vec<Round> = std::iter::once(r1.clone()).chain(r2.clone()).collect();
    let refs: Vec<Option<&Round>> = rounds.iter().map(Some).collect();
    let (verdict, severity, diff) = compare_rounds(Scenario::Anonymous, &refs);
    Some((rounds, verdict, severity, diff))
}

async fn run_cross_user(
    client: &reqwest::Client,
    ep: &Endpoint,
    cfg: &ProbeConfig,
    summary: &mut ProbeSummary,
) -> Option<(Vec<Round>, Verdict, Severity, String)> {
    if !ep.has_path_params {
        return None;
    }
    if cfg.id_pool_a.is_empty() || cfg.id_pool_b.is_empty() {
        return None;
    }
    if !matches!(cfg.token_a, TokenSource::Plain { .. })
        || !matches!(cfg.token_b, TokenSource::Plain { .. })
    {
        return None;
    }

    let id_a = cfg.id_pool_a.first()?.as_str();
    let id_b = cfg.id_pool_b.first()?.as_str();

    let path_a = substitute_id(ep, SubstituteKind::NewId { id: id_a })?;
    let path_b = substitute_id(ep, SubstituteKind::NewId { id: id_b })?;
    let url_a = join_url(&cfg.base_url, &path_a);
    let url_b = join_url(&cfg.base_url, &path_b);

    let r1 = execute_round(client, &ep.method, &url_a, &cfg.token_a).await;
    summary.total_requests += 1;
    bump_summary_for_round(&r1, summary);
    let r2 = execute_round(client, &ep.method, &url_b, &cfg.token_a).await;
    summary.total_requests += 1;
    bump_summary_for_round(&r2, summary);
    let r3 = execute_round(client, &ep.method, &url_b, &cfg.token_b).await;
    summary.total_requests += 1;
    bump_summary_for_round(&r3, summary);

    let rounds = vec![r1, r2, r3];
    let refs: Vec<Option<&Round>> = rounds.iter().map(Some).collect();
    let (verdict, severity, diff) = compare_rounds(Scenario::CrossUser, &refs);
    Some((rounds, verdict, severity, diff))
}

async fn run_privilege(
    client: &reqwest::Client,
    ep: &Endpoint,
    cfg: &ProbeConfig,
    summary: &mut ProbeSummary,
) -> Option<(Vec<Round>, Verdict, Severity, String)> {
    if !is_admin_shaped(&ep.path) {
        return None;
    }
    if !matches!(cfg.token_a, TokenSource::Plain { .. }) {
        return None;
    }
    let path = substitute_id(ep, SubstituteKind::SameId { default_id: "1" })?;
    let url = join_url(&cfg.base_url, &path);

    let r1 = execute_round(client, &ep.method, &url, &cfg.token_a).await;
    summary.total_requests += 1;
    bump_summary_for_round(&r1, summary);
    let r2 = execute_round(client, &ep.method, &url, &TokenSource::None).await;
    summary.total_requests += 1;
    bump_summary_for_round(&r2, summary);

    let rounds = vec![r1, r2];
    let refs: Vec<Option<&Round>> = rounds.iter().map(Some).collect();
    let (verdict, severity, diff) = compare_rounds(Scenario::Privilege, &refs);
    Some((rounds, verdict, severity, diff))
}

fn is_admin_shaped(path: &str) -> bool {
    let lower = path.to_lowercase();
    ["admin", "internal", "manage", "console", "operator"]
        .iter()
        .any(|kw| lower.contains(kw))
}

fn bump_summary_for_round(r: &Round, s: &mut ProbeSummary) {
    match r.outcome {
        RoundOutcome::RateLimited => s.rate_limited_count += 1,
        RoundOutcome::NetworkError => s.network_error_count += 1,
        _ => {}
    }
}

fn scenario_key(s: Scenario) -> String {
    match s {
        Scenario::Anonymous => "anonymous".into(),
        Scenario::CrossUser => "cross_user".into(),
        Scenario::Privilege => "privilege".into(),
    }
}

fn severity_key(sv: Severity) -> String {
    match sv {
        Severity::Critical => "critical".into(),
        Severity::High => "high".into(),
        Severity::Medium => "medium".into(),
        Severity::Low => "low".into(),
        Severity::Info => "info".into(),
    }
}

/// Concatenate `base_url` and `path` defensively. Either may carry a
/// leading/trailing `/`; we want exactly one separator between them.
fn join_url(base: &str, path: &str) -> String {
    let base_clean = base.trim_end_matches('/');
    let path_clean = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };
    format!("{}{}", base_clean, path_clean)
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_js_analyzer::{AuthHint, CallSiteKind, UrlKind};

    fn ep(method: &str, path: &str) -> Endpoint {
        Endpoint {
            method: method.into(),
            path: path.into(),
            auth: AuthHint::None,
            source_file: "test.js".into(),
            line: 1,
            confidence: 1.0,
            kind: CallSiteKind::Fetch,
            url_kind: UrlKind::Literal,
            has_path_params: false,
            id_param_position: None,
        }
    }

    #[test]
    fn safe_verbs_pass_default_policy() {
        assert!(is_probe_safe(&ep("GET", "/x"), false));
        assert!(is_probe_safe(&ep("HEAD", "/x"), false));
        assert!(is_probe_safe(&ep("OPTIONS", "/x"), false));
    }

    #[test]
    fn mutating_verbs_blocked_by_default() {
        assert!(!is_probe_safe(&ep("POST", "/x"), false));
        assert!(!is_probe_safe(&ep("PUT", "/x"), false));
        assert!(!is_probe_safe(&ep("DELETE", "/x"), false));
    }

    #[test]
    fn mutating_verbs_allowed_when_opted_in() {
        assert!(is_probe_safe(&ep("POST", "/x"), true));
        assert!(is_probe_safe(&ep("DELETE", "/x"), true));
    }

    #[test]
    fn admin_keywords_detected() {
        assert!(is_admin_shaped("/admin/users"));
        assert!(is_admin_shaped("/api/internal/config"));
        assert!(is_admin_shaped("/MANAGE/queue"));
        assert!(!is_admin_shaped("/api/users"));
    }

    #[test]
    fn join_url_dedupes_slashes() {
        assert_eq!(join_url("https://x.com", "/api"), "https://x.com/api");
        assert_eq!(join_url("https://x.com/", "/api"), "https://x.com/api");
        assert_eq!(join_url("https://x.com/", "api"), "https://x.com/api");
    }
}
