use super::candidates::{read_candidates_from_intel, upsert_candidates_into_intel};
use super::types::{OrganizationCandidate, OrganizationCandidateKind, OrganizationProfilePatch};
use super::validation::{is_valid_asn, is_valid_cidr, is_valid_domain, validate_profile_patch};

#[test]
fn cidr_validation_accepts_ipv4_and_ipv6() {
    assert!(is_valid_cidr("10.0.0.0/8"));
    assert!(is_valid_cidr("192.168.1.0/24"));
    assert!(is_valid_cidr("0.0.0.0/0"));
    assert!(is_valid_cidr("2001:db8::/32"));
    assert!(is_valid_cidr("::/0"));
}

#[test]
fn cidr_validation_rejects_garbage() {
    assert!(!is_valid_cidr(""));
    assert!(!is_valid_cidr("10.0.0.0"));
    assert!(!is_valid_cidr("10.0.0.0/"));
    assert!(!is_valid_cidr("10.0.0.0/33"));
    assert!(!is_valid_cidr("10.0.0.0/abc"));
    assert!(!is_valid_cidr("not-an-ip/24"));
    assert!(!is_valid_cidr("2001:db8::/129"));
}

#[test]
fn domain_validation_accepts_normal_and_wildcard() {
    assert!(is_valid_domain("example.com"));
    assert!(is_valid_domain("a.b.example.com"));
    assert!(is_valid_domain("*.example.com"));
    assert!(is_valid_domain("xn--80akhbyknj4f.com"));
}

#[test]
fn domain_validation_rejects_garbage() {
    assert!(!is_valid_domain(""));
    assert!(!is_valid_domain("example"));
    assert!(!is_valid_domain(".example.com"));
    assert!(!is_valid_domain("example..com"));
    assert!(!is_valid_domain("-bad.com"));
    assert!(!is_valid_domain("bad-.com"));
}

#[test]
fn asn_validation() {
    assert!(is_valid_asn("AS1"));
    assert!(is_valid_asn("AS12345"));
    assert!(!is_valid_asn(""));
    assert!(!is_valid_asn("12345"));
    assert!(!is_valid_asn("as12345"));
    assert!(!is_valid_asn("AS"));
    assert!(!is_valid_asn("AS12345678901"));
}

#[test]
fn validate_patch_collects_all_errors() {
    let p = OrganizationProfilePatch {
        tier: Some("supreme".into()),
        ip_ranges: Some(serde_json::json!(["10.0.0.0/24", "bad-ip"])),
        asns: Some(serde_json::json!(["AS1", "not-an-asn"])),
        domains: Some(serde_json::json!(["good.com", "bad..com"])),
        email_domains: Some(serde_json::json!(["pingan.com", "x x"])),
        ..Default::default()
    };
    let errs = validate_profile_patch(&p);
    let fields: Vec<&str> = errs.iter().map(|(f, _, _)| f.as_str()).collect();
    assert!(fields.contains(&"tier"));
    assert!(fields.contains(&"ip_ranges"));
    assert!(fields.contains(&"asns"));
    assert!(fields.contains(&"domains"));
    assert!(fields.contains(&"email_domains"));
}

#[test]
fn validate_patch_accepts_clean_payload() {
    let p = OrganizationProfilePatch {
        tier: Some("critical".into()),
        ip_ranges: Some(serde_json::json!(["10.0.0.0/24", "2001:db8::/32"])),
        asns: Some(serde_json::json!(["AS12345"])),
        domains: Some(serde_json::json!(["pingan.com", "*.pingan.com"])),
        email_domains: Some(serde_json::json!(["pingan.com"])),
        ..Default::default()
    };
    assert!(validate_profile_patch(&p).is_empty());
}

#[test]
fn profile_patch_deserializes_snake_case_keys() {
    // The frontend (lib/api/organizations.ts) sends snake_case patch keys;
    // they must deserialize so multi-word fields actually update (regression
    // guard for the old `rename_all = "camelCase"` that silently dropped them).
    let json = serde_json::json!({
        "credit_code": "91110108551385082Q",
        "ip_ranges": ["10.0.0.0/24"],
        "email_domains": ["pingan.com"],
        "scope_rules": { "in": ["example.com"] },
    });
    let p: OrganizationProfilePatch = serde_json::from_value(json).unwrap();
    assert_eq!(p.credit_code.as_deref(), Some("91110108551385082Q"));
    assert!(p.ip_ranges.is_some());
    assert!(p.email_domains.is_some());
    assert!(p.scope_rules.is_some());
}

#[test]
fn candidate_upsert_dedupes_by_id_and_preserves_engagement() {
    let intel = serde_json::json!({
        "engagement": {
            "mode": "discover_assets",
            "candidates": {
                "targets": [
                    {
                        "id": "target:seed:old.example.com",
                        "kind": "target",
                        "label": "old",
                        "value": "old.example.com",
                        "source": "seed",
                        "confidence": 0.5,
                        "status": "needs_review",
                        "evidence": {},
                        "createdAt": 1
                    }
                ]
            }
        }
    });
    let updated = upsert_candidates_into_intel(
        intel,
        vec![
            OrganizationCandidate {
                id: "target:seed:old.example.com".into(),
                kind: OrganizationCandidateKind::Target,
                label: "old updated".into(),
                value: "old.example.com".into(),
                source: "seed".into(),
                confidence: 0.9,
                status: "approved".into(),
                evidence: serde_json::json!({"reason": "customer confirmed"}),
                created_at: 2,
            },
            OrganizationCandidate {
                id: "".into(),
                kind: OrganizationCandidateKind::Organization,
                label: "Subsidiary A".into(),
                value: "Subsidiary A".into(),
                source: "enscan".into(),
                confidence: 0.8,
                status: "".into(),
                evidence: serde_json::json!({"ownership": 51}),
                created_at: 0,
            },
        ],
    )
    .expect("candidate upsert should succeed");
    let store = read_candidates_from_intel(&updated);
    assert_eq!(
        updated["engagement"]["mode"],
        serde_json::Value::String("discover_assets".into())
    );
    assert_eq!(store.targets.len(), 1);
    assert_eq!(store.targets[0].label, "old updated");
    assert_eq!(store.organizations.len(), 1);
    assert_eq!(store.organizations[0].status, "needs_review");
    assert!(store.organizations[0].id.starts_with("org:enscan:"));
}
