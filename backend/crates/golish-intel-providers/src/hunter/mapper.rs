//! Map Hunter rows to uniform [`ProviderRecord`]s.

use std::collections::HashMap;

use crate::types::{ProviderRecord, QueryType};

use super::types::HunterRow;

const PROVIDER: &str = "hunter";

fn insert_if_present(fields: &mut HashMap<String, String>, key: &str, val: Option<&String>) {
    if let Some(v) = val {
        if !v.is_empty() {
            fields.insert(key.to_string(), v.clone());
        }
    }
}

/// `Site` mapping — Hunter's primary query type. Surfaces:
/// - `url` / `ip` / `port` / `domain` / `protocol` / `title` / `status_code`
/// - `organization_name` (from Hunter's `company` field — used to find/create org)
/// - `webserver` / `os` / `isp` / fingerprint hints from `component[]`
pub fn map_site(row: HunterRow, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = HashMap::new();
    insert_if_present(&mut fields, "url", row.url.as_ref());
    insert_if_present(&mut fields, "ip", row.ip.as_ref());
    if let Some(p) = row.port {
        fields.insert("port".into(), p.to_string());
    }
    insert_if_present(&mut fields, "domain", row.domain.as_ref());
    insert_if_present(&mut fields, "protocol", row.protocol.as_ref());
    insert_if_present(&mut fields, "title", row.web_title.as_ref());
    if let Some(sc) = row.status_code {
        fields.insert("status_code".into(), sc.to_string());
    }
    insert_if_present(&mut fields, "organization_name", row.company.as_ref());
    insert_if_present(&mut fields, "os", row.os.as_ref());
    insert_if_present(&mut fields, "isp", row.isp.as_ref());
    insert_if_present(&mut fields, "country", row.country.as_ref());
    insert_if_present(&mut fields, "asn_org", row.as_org.as_ref());
    insert_if_present(&mut fields, "base_protocol", row.base_protocol.as_ref());
    insert_if_present(&mut fields, "cert", row.ssl_certificate.as_ref());
    insert_if_present(&mut fields, "cert_sha256", row.cert_sha256.as_ref());
    insert_if_present(&mut fields, "beian", row.icp_number.as_ref());

    // Pick the first component as the canonical webserver fingerprint.
    if let Some(first) = row.component.first() {
        if let Some(name) = &first.name {
            if !name.is_empty() {
                let value = match (&first.name, &first.version) {
                    (Some(n), Some(v)) if !v.is_empty() => format!("{n}/{v}"),
                    _ => name.clone(),
                };
                fields.insert("webserver".into(), value);
            }
        }
    }
    if !fields.contains_key("webserver") {
        insert_if_present(&mut fields, "webserver", row.header_server.as_ref());
    }

    ProviderRecord::new(PROVIDER, QueryType::Site, fields, raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunter::types::HunterComponent;

    fn raw_object() -> serde_json::Value {
        serde_json::json!({})
    }

    #[test]
    fn site_mapper_extracts_all_fields() {
        let row = HunterRow {
            url: Some("https://x.com".into()),
            ip: Some("1.2.3.4".into()),
            port: Some(443),
            domain: Some("x.com".into()),
            protocol: Some("https".into()),
            web_title: Some("Hi".into()),
            status_code: Some(200),
            component: vec![HunterComponent {
                name: Some("nginx".into()),
                version: Some("1.21.4".into()),
            }],
            os: Some("Linux".into()),
            country: Some("CN".into()),
            province: Some("Beijing".into()),
            city: Some("Beijing".into()),
            company: Some("Acme Corp".into()),
            isp: Some("ChinaTelecom".into()),
            updated_at: None,
            header_server: None,
            as_org: None,
            base_protocol: None,
            ssl_certificate: None,
            cert_sha256: None,
            icp_number: None,
        };
        let rec = map_site(row, raw_object());
        assert_eq!(rec.provider, "hunter");
        assert_eq!(rec.query_type, QueryType::Site);
        assert_eq!(rec.fields.get("ip").unwrap(), "1.2.3.4");
        assert_eq!(rec.fields.get("port").unwrap(), "443");
        assert_eq!(rec.fields.get("organization_name").unwrap(), "Acme Corp");
        assert_eq!(rec.fields.get("webserver").unwrap(), "nginx/1.21.4");
        assert_eq!(rec.fields.get("isp").unwrap(), "ChinaTelecom");
    }

    #[test]
    fn site_mapper_handles_missing_version() {
        let row = HunterRow {
            component: vec![HunterComponent {
                name: Some("apache".into()),
                version: None,
            }],
            ..Default::default()
        };
        let rec = map_site(row, raw_object());
        assert_eq!(rec.fields.get("webserver").unwrap(), "apache");
    }

    #[test]
    fn site_mapper_skips_empty_component() {
        let row = HunterRow {
            component: vec![HunterComponent {
                name: Some("".into()),
                version: None,
            }],
            ..Default::default()
        };
        let rec = map_site(row, raw_object());
        assert!(!rec.fields.contains_key("webserver"));
    }

    #[test]
    fn site_mapper_uses_real_hunter_extra_fields() {
        let row = HunterRow {
            url: Some("https://vpn.baidu.com".into()),
            ip: Some("111.45.11.188".into()),
            port: Some(443),
            domain: Some("vpn.baidu.com".into()),
            protocol: Some("https".into()),
            web_title: Some("VPN".into()),
            status_code: Some(200),
            component: vec![],
            os: Some("Linux".into()),
            country: Some("中国".into()),
            province: Some("广东省".into()),
            city: Some("广州市".into()),
            company: Some("北京百度网讯科技有限公司".into()),
            isp: Some("中国移动".into()),
            updated_at: Some("2026-05-21".into()),
            header_server: Some("nginx".into()),
            as_org: Some("China Mobile communications corporation".into()),
            base_protocol: Some("tcp".into()),
            ssl_certificate: Some("Subject: CN=vpn.baidu.com".into()),
            cert_sha256: Some("abc123".into()),
            icp_number: Some("京ICP证030173号".into()),
        };
        let rec = map_site(row, raw_object());
        assert_eq!(
            rec.fields.get("webserver").map(String::as_str),
            Some("nginx")
        );
        assert_eq!(
            rec.fields.get("asn_org").map(String::as_str),
            Some("China Mobile communications corporation")
        );
        assert_eq!(
            rec.fields.get("cert").map(String::as_str),
            Some("Subject: CN=vpn.baidu.com")
        );
        assert_eq!(
            rec.fields.get("beian").map(String::as_str),
            Some("京ICP证030173号")
        );
        assert_eq!(
            rec.fields.get("base_protocol").map(String::as_str),
            Some("tcp")
        );
    }

    #[test]
    fn empty_row_yields_empty_fields() {
        let rec = map_site(HunterRow::default(), raw_object());
        assert!(rec.fields.is_empty());
    }
}
