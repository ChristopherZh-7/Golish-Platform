//! Tests for pure helper functions in golish-vuln-intel.
//!
//! These cover header construction, default feed catalog, severity
//! extraction, NVD URL builder, and HTTP client construction. Network
//! and DB-touching paths are out of scope and need a Postgres harness.

use golish_vuln_intel::{
    build_github_client, default_feeds, extract_nuclei_severity, github_headers, nvd_recent_url,
    VulnFeed,
};

#[test]
fn github_headers_without_token_omits_authorization() {
    let h = github_headers(&None);
    assert_eq!(h.get("User-Agent").unwrap(), "golish-platform");
    assert!(h.contains_key("Accept"));
    assert!(!h.contains_key("Authorization"));
}

#[test]
fn github_headers_with_token_includes_bearer() {
    let h = github_headers(&Some("ghp_secret".into()));
    let auth = h.get("Authorization").unwrap().to_str().unwrap();
    assert_eq!(auth, "Bearer ghp_secret");
}

#[test]
fn build_github_client_without_proxy_succeeds() {
    let client = build_github_client(None).expect("client builds");
    // Smoke test: client is usable for normal calls (we won't actually fire one).
    drop(client);
}

#[test]
fn build_github_client_with_empty_proxy_still_succeeds() {
    let client = build_github_client(Some("")).expect("empty proxy ignored");
    drop(client);
}

#[test]
fn default_feeds_includes_cisa_and_nvd() {
    let feeds = default_feeds();
    let ids: Vec<&str> = feeds.iter().map(|f: &VulnFeed| f.id.as_str()).collect();
    assert!(ids.contains(&"cisa-kev"), "CISA KEV must be a default feed");
    assert!(ids.contains(&"nvd-recent"), "NVD recent must be a default feed");

    // Every feed has a non-empty feed_type and a non-empty id.
    for f in &feeds {
        assert!(!f.feed_type.is_empty(), "feed_type required for {}", f.id);
        assert!(!f.id.is_empty(), "id required");
    }

    // Core feeds (CISA KEV + NVD recent) must be enabled by default; optional
    // RSS feeds (e.g. cnvd) may ship disabled.
    let by_id = |id: &str| feeds.iter().find(|f| f.id == id).expect("feed present");
    assert!(by_id("cisa-kev").enabled, "CISA KEV must be enabled by default");
    assert!(by_id("nvd-recent").enabled, "NVD recent must be enabled by default");
}

#[test]
fn extract_nuclei_severity_finds_field() {
    let yaml = "id: CVE-2021-44228\ninfo:\n  name: log4shell\n  severity: critical\n";
    assert_eq!(extract_nuclei_severity(yaml).as_deref(), Some("critical"));
}

#[test]
fn extract_nuclei_severity_returns_none_when_missing() {
    assert!(extract_nuclei_severity("id: foo\nname: bar\n").is_none());
}

#[test]
fn extract_nuclei_severity_strips_whitespace() {
    let yaml = "  severity:    high  ";
    assert_eq!(extract_nuclei_severity(yaml).as_deref(), Some("high"));
}

#[test]
fn nvd_recent_url_returns_https_endpoint_with_window() {
    let url = nvd_recent_url(7);
    assert!(url.starts_with("https://"), "must be https");
    assert!(url.contains("nvd"), "must reference NVD");
    assert!(url.contains("pubStartDate"), "must include pubStartDate");
    assert!(url.contains("pubEndDate"), "must include pubEndDate");
}
