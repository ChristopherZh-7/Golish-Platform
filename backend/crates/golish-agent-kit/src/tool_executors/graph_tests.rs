use super::*;

#[test]
fn extracts_ipv4_addresses() {
    let txt = "Found host 10.0.0.5 and gateway 192.168.1.1.";
    let out = extract_kg_candidates(txt);
    let hosts: Vec<&str> = out
        .iter()
        .filter(|(t, _)| t == "host")
        .map(|(_, n)| n.as_str())
        .collect();
    assert!(hosts.contains(&"10.0.0.5"));
    assert!(hosts.contains(&"192.168.1.1"));
}

#[test]
fn skips_placeholder_addresses() {
    let txt = "0.0.0.0:80 listener and broadcast 255.255.255.255";
    let out = extract_kg_candidates(txt);
    let hosts: Vec<&str> = out
        .iter()
        .filter(|(t, _)| t == "host")
        .map(|(_, n)| n.as_str())
        .collect();
    assert!(!hosts.contains(&"0.0.0.0"));
    assert!(!hosts.contains(&"255.255.255.255"));
}

#[test]
fn rejects_invalid_octet_ranges() {
    // 999.x.x.x is not a valid IPv4 — must not appear
    let txt = "version 999.1.2.3 in changelog and host 10.0.0.5";
    let out = extract_kg_candidates(txt);
    let hosts: Vec<&str> = out
        .iter()
        .filter(|(t, _)| t == "host")
        .map(|(_, n)| n.as_str())
        .collect();
    assert!(hosts.contains(&"10.0.0.5"));
    assert!(!hosts.iter().any(|h| h.starts_with("999.")));
}

#[test]
fn extracts_cve_identifiers_uppercase() {
    let txt = "vulnerable to cve-2024-1234 and CVE-2023-99999";
    let out = extract_kg_candidates(txt);
    let cves: Vec<&str> = out
        .iter()
        .filter(|(t, _)| t == "vulnerability")
        .map(|(_, n)| n.as_str())
        .collect();
    assert!(cves.contains(&"CVE-2024-1234"));
    assert!(cves.contains(&"CVE-2023-99999"));
}

#[test]
fn extracts_https_url_without_trailing_punctuation() {
    let txt = "see https://example.com/admin/login. The endpoint matters.";
    let out = extract_kg_candidates(txt);
    let urls: Vec<&str> = out
        .iter()
        .filter(|(t, _)| t == "endpoint")
        .map(|(_, n)| n.as_str())
        .collect();
    assert!(urls.contains(&"https://example.com/admin/login"));
}

#[test]
fn deduplicates_within_one_pass() {
    let txt = "host 10.0.0.5 then 10.0.0.5 again";
    let out = extract_kg_candidates(txt);
    let hosts: Vec<&str> = out
        .iter()
        .filter(|(t, _)| t == "host")
        .map(|(_, n)| n.as_str())
        .collect();
    assert_eq!(hosts.iter().filter(|h| **h == "10.0.0.5").count(), 1);
}

#[test]
fn returns_empty_for_pure_prose() {
    let out = extract_kg_candidates("This is a sentence with no IPs CVEs or URLs.");
    assert!(out.is_empty());
}

// --- co-occurrence edge derivation (option A) ---

use uuid::Uuid;

fn ent(t: &str, id: Uuid, name: &str) -> (String, Uuid, String) {
    (t.to_string(), id, name.to_string())
}

#[test]
fn host_from_url_strips_scheme_port_userinfo_path() {
    assert_eq!(
        host_from_url("http://10.0.0.5:8080/admin"),
        Some("10.0.0.5".to_string())
    );
    assert_eq!(
        host_from_url("https://a.com/x?q=1#frag"),
        Some("a.com".to_string())
    );
    assert_eq!(
        host_from_url("https://user:pw@h.example/path"),
        Some("h.example".to_string())
    );
    assert_eq!(host_from_url("not a url with spaces"), None);
    assert_eq!(host_from_url(""), None);
}

#[test]
fn plan_edges_empty_without_any_host() {
    let v = vec![
        ent("vulnerability", Uuid::new_v4(), "CVE-2021-1"),
        ent("endpoint", Uuid::new_v4(), "https://a.com/x"),
    ];
    assert!(plan_cooccurrence_edges(&v).is_empty());
}

#[test]
fn plan_edges_host_vuln_is_weak_cooccurrence() {
    let h = Uuid::new_v4();
    let vuln = Uuid::new_v4();
    let edges = plan_cooccurrence_edges(&[ent("host", h, "10.0.0.5"), ent("vulnerability", vuln, "CVE-2021-1")]);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].from, h);
    assert_eq!(edges[0].to, vuln);
    assert_eq!(edges[0].rel, "has_vulnerability");
    assert_eq!(edges[0].source, "cooccurrence");
    assert_eq!(edges[0].confidence, "low");
}

#[test]
fn plan_edges_url_parse_is_strong_when_host_matches() {
    let h = Uuid::new_v4();
    let ep = Uuid::new_v4();
    let edges = plan_cooccurrence_edges(&[
        ent("host", h, "10.0.0.5"),
        ent("endpoint", ep, "http://10.0.0.5:8080/admin"),
    ]);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].from, h);
    assert_eq!(edges[0].to, ep);
    assert_eq!(edges[0].rel, "exposes_endpoint");
    assert_eq!(edges[0].source, "url_parse");
    assert_eq!(edges[0].confidence, "high");
}

#[test]
fn plan_edges_url_falls_back_to_cooccurrence_when_no_host_match() {
    let h = Uuid::new_v4();
    let ep = Uuid::new_v4();
    let edges = plan_cooccurrence_edges(&[
        ent("host", h, "10.0.0.5"),
        ent("endpoint", ep, "https://other.example/x"),
    ]);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].rel, "exposes_endpoint");
    assert_eq!(edges[0].source, "cooccurrence");
    assert_eq!(edges[0].confidence, "low");
}

#[test]
fn plan_edges_two_hosts_one_vuln_yields_two_edges() {
    let h1 = Uuid::new_v4();
    let h2 = Uuid::new_v4();
    let vuln = Uuid::new_v4();
    let edges = plan_cooccurrence_edges(&[
        ent("host", h1, "10.0.0.1"),
        ent("host", h2, "10.0.0.2"),
        ent("vulnerability", vuln, "CVE-2021-1"),
    ]);
    assert_eq!(edges.len(), 2);
    assert!(edges.iter().all(|e| e.rel == "has_vulnerability" && e.to == vuln));
}

#[test]
fn plan_edges_are_capped() {
    let h = Uuid::new_v4();
    let mut v = vec![ent("host", h, "10.0.0.5")];
    for i in 0..(MAX_COOC_EDGES + 10) {
        v.push(ent("vulnerability", Uuid::new_v4(), &format!("CVE-2021-{i}")));
    }
    assert_eq!(plan_cooccurrence_edges(&v).len(), MAX_COOC_EDGES);
}
