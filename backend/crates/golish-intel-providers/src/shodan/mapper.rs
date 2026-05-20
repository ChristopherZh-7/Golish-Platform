//! Map Shodan matches to uniform [`ProviderRecord`]s.

use std::collections::HashMap;

use crate::types::{ProviderRecord, QueryType};

use super::types::ShodanMatch;

const PROVIDER: &str = "shodan";

fn insert_if_present(fields: &mut HashMap<String, String>, key: &str, val: Option<&String>) {
    if let Some(v) = val {
        if !v.is_empty() {
            fields.insert(key.to_string(), v.clone());
        }
    }
}

/// `Site` mapping — banner-per-port surface area.
///
/// Field keys emitted (subset depending on which fields the match has):
/// - `ip` / `port` / `transport` — service identity
/// - `organization_name` (from Shodan `org`)
/// - `isp` / `asn` / `os` — fingerprints
/// - `country` — location
/// - `domain` — first hostname / domain in the match (writer picks the
///   first one to push into `organizations.domains`)
/// - `cert` — TLS subject CN if present (push into `organizations.certificates`)
/// - `webserver` / `title` / `status_code` — only when `http` is set
pub fn map_site(m: ShodanMatch, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    insert_if_present(&mut fields, "ip", m.ip_str.as_ref());
    if let Some(p) = m.port {
        fields.insert("port".into(), p.to_string());
    }
    insert_if_present(&mut fields, "transport", m.transport.as_ref());
    insert_if_present(&mut fields, "organization_name", m.org.as_ref());
    insert_if_present(&mut fields, "isp", m.isp.as_ref());
    insert_if_present(&mut fields, "asn", m.asn.as_ref());
    insert_if_present(&mut fields, "os", m.os.as_ref());

    if let Some(loc) = &m.location {
        insert_if_present(&mut fields, "country", loc.country_code.as_ref());
    }

    // Push the most-specific domain — hostnames before domains.
    let host = m
        .hostnames
        .iter()
        .find(|h| !h.is_empty())
        .or_else(|| m.domains.iter().find(|d| !d.is_empty()));
    if let Some(h) = host {
        fields.insert("domain".into(), h.clone());
    }

    // HTTP-layer fingerprint when present.
    if let Some(http) = &m.http {
        insert_if_present(&mut fields, "webserver", http.server.as_ref());
        insert_if_present(&mut fields, "title", http.title.as_ref());
        if let Some(sc) = http.status {
            fields.insert("status_code".into(), sc.to_string());
        }
    }

    // SSL cert subject CN → `cert`.
    if let Some(ssl) = &m.ssl {
        if let Some(cert) = &ssl.cert {
            if let Some(subj) = &cert.subject {
                insert_if_present(&mut fields, "cert", subj.cn.as_ref());
            }
        }
    }

    ProviderRecord::new(PROVIDER, QueryType::Site, fields, raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shodan::types::{
        ShodanCert, ShodanCertSubject, ShodanHttp, ShodanLocation, ShodanSsl,
    };

    fn raw_object() -> serde_json::Value {
        serde_json::json!({})
    }

    #[test]
    fn site_mapper_basic_fields() {
        let m = ShodanMatch {
            ip_str: Some("1.2.3.4".into()),
            port: Some(443),
            org: Some("Acme".into()),
            isp: Some("ChinaTelecom".into()),
            asn: Some("AS13335".into()),
            transport: Some("tcp".into()),
            location: Some(ShodanLocation {
                country_code: Some("US".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let rec = map_site(m, raw_object());
        assert_eq!(rec.provider, "shodan");
        assert_eq!(rec.query_type, QueryType::Site);
        assert_eq!(rec.fields.get("ip").unwrap(), "1.2.3.4");
        assert_eq!(rec.fields.get("port").unwrap(), "443");
        assert_eq!(rec.fields.get("organization_name").unwrap(), "Acme");
        assert_eq!(rec.fields.get("asn").unwrap(), "AS13335");
        assert_eq!(rec.fields.get("country").unwrap(), "US");
    }

    #[test]
    fn site_mapper_picks_hostname_over_domain() {
        let m = ShodanMatch {
            hostnames: vec!["api.example.com".into()],
            domains: vec!["example.com".into()],
            ..Default::default()
        };
        let rec = map_site(m, raw_object());
        assert_eq!(rec.fields.get("domain").unwrap(), "api.example.com");
    }

    #[test]
    fn site_mapper_falls_back_to_domain_when_no_hostname() {
        let m = ShodanMatch {
            hostnames: vec!["".into()],
            domains: vec!["example.com".into()],
            ..Default::default()
        };
        let rec = map_site(m, raw_object());
        assert_eq!(rec.fields.get("domain").unwrap(), "example.com");
    }

    #[test]
    fn site_mapper_extracts_http_block() {
        let m = ShodanMatch {
            http: Some(ShodanHttp {
                server: Some("nginx".into()),
                title: Some("hello".into()),
                status: Some(200),
                host: None,
            }),
            ..Default::default()
        };
        let rec = map_site(m, raw_object());
        assert_eq!(rec.fields.get("webserver").unwrap(), "nginx");
        assert_eq!(rec.fields.get("title").unwrap(), "hello");
        assert_eq!(rec.fields.get("status_code").unwrap(), "200");
    }

    #[test]
    fn site_mapper_extracts_ssl_cert_cn() {
        let m = ShodanMatch {
            ssl: Some(ShodanSsl {
                cert: Some(ShodanCert {
                    subject: Some(ShodanCertSubject {
                        cn: Some("example.com".into()),
                    }),
                    issuer: None,
                }),
            }),
            ..Default::default()
        };
        let rec = map_site(m, raw_object());
        assert_eq!(rec.fields.get("cert").unwrap(), "example.com");
    }

    #[test]
    fn empty_match_yields_empty_fields() {
        let rec = map_site(ShodanMatch::default(), raw_object());
        assert!(rec.fields.is_empty());
    }
}
