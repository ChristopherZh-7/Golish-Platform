//! Map 360 Quake `QuakeService` records to uniform [`ProviderRecord`]s.
//!
//! Quake responses are deeply nested (`service.http.title`, `location.city_cn`,
//! ...). The mapper flattens the bits that matter into normalized field keys
//! that `output_store::store_organization_update` can route.

use std::collections::HashMap;

use crate::types::{ProviderRecord, QueryType};

use super::types::QuakeService;

const PROVIDER: &str = "quake";

fn insert_if_present(fields: &mut HashMap<String, String>, key: &str, val: Option<&String>) {
    if let Some(v) = val {
        if !v.is_empty() {
            fields.insert(key.to_string(), v.clone());
        }
    }
}

fn base_fields(svc: &QuakeService) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    insert_if_present(&mut fields, "ip", svc.ip.as_ref());
    if let Some(port) = svc.port {
        fields.insert("port".into(), port.to_string());
    }
    insert_if_present(&mut fields, "transport", svc.transport.as_ref());
    insert_if_present(&mut fields, "org", svc.org.as_ref());
    if let Some(asn) = svc.asn_string() {
        fields.insert("asn".into(), asn);
    }
    if let Some(loc) = &svc.location {
        insert_if_present(&mut fields, "country", loc.country_en.as_ref());
        insert_if_present(&mut fields, "country_cn", loc.country_cn.as_ref());
        insert_if_present(&mut fields, "city_cn", loc.city_cn.as_ref());
        insert_if_present(&mut fields, "isp", loc.isp.as_ref());
    }
    if let Some(inner) = &svc.service {
        insert_if_present(&mut fields, "service_name", inner.name.as_ref());
        if let Some(http) = &inner.http {
            insert_if_present(&mut fields, "title", http.title.as_ref());
            insert_if_present(&mut fields, "server", http.server.as_ref());
            insert_if_present(&mut fields, "http_host", http.host.as_ref());
        }
    }
    fields
}

/// Quake nests the cert subject under `service.cert` (v3 `quake_service`) but
/// "sometimes flattens it" to the top level. Prefer flat, fall back to nested,
/// so CT lands regardless of shape (2026-06-23 live: nested was being dropped →
/// all Quake CT silently lost).
fn cert_subject(svc: &QuakeService) -> Option<String> {
    svc.cert
        .clone()
        .filter(|c| !c.is_empty())
        .or_else(|| {
            svc.service
                .as_ref()
                .and_then(|s| s.cert.clone())
                .filter(|c| !c.is_empty())
        })
}

/// `Site` mapping — full service surface (ip/port/protocol/title/...).
pub fn map_site(svc: QuakeService, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = base_fields(&svc);
    // When Quake surfaces a domain in the service record, route it so the
    // writer can append into organizations.domains.
    let domain = svc.domain.clone().or_else(|| svc.hostname.clone());
    insert_if_present(&mut fields, "domain", domain.as_ref());
    insert_if_present(&mut fields, "cert", cert_subject(&svc).as_ref());
    ProviderRecord::new(PROVIDER, QueryType::Site, fields, raw)
}

/// `Domain` mapping — Quake-specific `quake_service` doesn't have a separate
/// subdomain endpoint, but our query renders `domain: "..."` and the response
/// carries the matching subdomain in `domain` or `service.http.host`. Always
/// emit a `domain` field key so the writer can route into
/// `organizations.domains`.
pub fn map_domain(svc: QuakeService, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = base_fields(&svc);
    let domain = svc
        .domain
        .clone()
        .or_else(|| svc.hostname.clone())
        .or_else(|| {
            svc.service
                .as_ref()
                .and_then(|s| s.http.as_ref())
                .and_then(|h| h.host.clone())
        });
    insert_if_present(&mut fields, "domain", domain.as_ref());
    ProviderRecord::new(PROVIDER, QueryType::Domain, fields, raw)
}

/// `Cert` mapping — surface certificate info into `organizations.certificates`.
pub fn map_cert(svc: QuakeService, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = base_fields(&svc);
    insert_if_present(&mut fields, "cert", cert_subject(&svc).as_ref());
    insert_if_present(&mut fields, "domain", svc.domain.as_ref());
    ProviderRecord::new(PROVIDER, QueryType::Cert, fields, raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quake::types::{QuakeHttp, QuakeInnerService, QuakeLocation};

    fn sample_service() -> QuakeService {
        QuakeService {
            ip: Some("1.2.3.4".into()),
            port: Some(443),
            domain: Some("example.com".into()),
            hostname: Some("www.example.com".into()),
            transport: Some("tcp".into()),
            asn: Some(serde_json::json!(4134)),
            org: Some("Cogent".into()),
            location: Some(QuakeLocation {
                country_cn: Some("中国".into()),
                country_en: Some("China".into()),
                province_cn: Some("北京".into()),
                city_cn: Some("北京".into()),
                isp: Some("China Telecom".into()),
            }),
            service: Some(QuakeInnerService {
                name: Some("http".into()),
                http: Some(QuakeHttp {
                    title: Some("Hello".into()),
                    host: Some("example.com".into()),
                    server: Some("nginx".into()),
                }),
                tls: None,
                cert: None,
            }),
            cert: Some("CN=*.example.com".into()),
        }
    }

    fn raw() -> serde_json::Value {
        serde_json::json!({})
    }

    #[test]
    fn site_mapper_extracts_full_surface() {
        let rec = map_site(sample_service(), raw());
        assert_eq!(rec.provider, "quake");
        assert_eq!(rec.query_type, QueryType::Site);
        assert_eq!(rec.fields.get("ip").unwrap(), "1.2.3.4");
        assert_eq!(rec.fields.get("port").unwrap(), "443");
        assert_eq!(rec.fields.get("title").unwrap(), "Hello");
        assert_eq!(rec.fields.get("server").unwrap(), "nginx");
        assert_eq!(rec.fields.get("asn").unwrap(), "AS4134");
        assert_eq!(rec.fields.get("country").unwrap(), "China");
        assert_eq!(rec.fields.get("city_cn").unwrap(), "北京");
        assert_eq!(rec.fields.get("isp").unwrap(), "China Telecom");
        assert_eq!(rec.fields.get("service_name").unwrap(), "http");
        assert_eq!(rec.fields.get("domain").unwrap(), "example.com");
        assert_eq!(rec.fields.get("cert").unwrap(), "CN=*.example.com");
    }

    #[test]
    fn domain_mapper_uses_domain_first() {
        let svc = sample_service();
        let rec = map_domain(svc, raw());
        assert_eq!(rec.fields.get("domain").unwrap(), "example.com");
    }

    #[test]
    fn domain_mapper_falls_back_to_hostname_then_http_host() {
        let svc = QuakeService {
            hostname: Some("api.example.com".into()),
            ..Default::default()
        };
        let rec = map_domain(svc, raw());
        assert_eq!(rec.fields.get("domain").unwrap(), "api.example.com");

        let svc = QuakeService {
            service: Some(QuakeInnerService {
                http: Some(QuakeHttp {
                    host: Some("cdn.example.com".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let rec = map_domain(svc, raw());
        assert_eq!(rec.fields.get("domain").unwrap(), "cdn.example.com");
    }

    #[test]
    fn cert_mapper_focuses_on_cert_and_domain() {
        let rec = map_cert(sample_service(), raw());
        assert_eq!(rec.query_type, QueryType::Cert);
        assert!(rec.fields.get("cert").unwrap().contains("CN=*.example.com"));
        assert_eq!(rec.fields.get("domain").unwrap(), "example.com");
    }

    #[test]
    fn cert_mapper_reads_nested_service_cert_when_flat_absent() {
        // Quake v3 quake_service nests the cert under `service.cert`; the record
        // has NO flat top-level `cert` (2026-06-23 live structure probe). map_cert
        // must still extract it — this drop is why Quake CT never landed.
        let svc = QuakeService {
            cert: None,
            domain: Some("pingan.com".into()),
            service: Some(QuakeInnerService {
                cert: Some("CN=*.pingan.com".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let rec = map_cert(svc, raw());
        assert_eq!(
            rec.fields.get("cert").map(String::as_str),
            Some("CN=*.pingan.com")
        );
        assert_eq!(rec.fields.get("domain").unwrap(), "pingan.com");
    }

    #[test]
    fn site_mapper_reads_nested_service_cert() {
        // Same nested-cert path on the Site mapper (provider survey records also
        // carry certs nested under service.cert).
        let svc = QuakeService {
            cert: None,
            ip: Some("1.2.3.4".into()),
            service: Some(QuakeInnerService {
                cert: Some("CN=svc.example".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let rec = map_site(svc, raw());
        assert_eq!(rec.fields.get("cert").map(String::as_str), Some("CN=svc.example"));
    }

    #[test]
    fn empty_service_yields_minimal_fields() {
        let svc = QuakeService::default();
        let rec = map_site(svc, raw());
        // No location / service / ip → fields must be empty (or only contain values we definitely populated; none here).
        assert!(rec.fields.is_empty());
    }
}
