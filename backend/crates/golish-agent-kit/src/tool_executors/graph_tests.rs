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
