//! Map FOFA wire-format rows to uniform [`ProviderRecord`]s.
//!
//! FOFA returns positional rows; [`super::types::FofaEnvelope::row`] turns
//! each `Vec<String>` into a [`FofaRow`] keyed by name. From there we
//! flatten to the normalized field keys expected by
//! `output_store::store_organization_update`.

use std::collections::HashMap;

use crate::types::{ProviderRecord, QueryType};

use super::types::FofaRow;

const PROVIDER: &str = "fofa";

fn insert_if_present(fields: &mut HashMap<String, String>, key: &str, val: Option<&String>) {
    if let Some(v) = val {
        if !v.is_empty() {
            fields.insert(key.to_string(), v.clone());
        }
    }
}

/// Common subset of fields shared by every FOFA mapper.
fn base_fields(row: &FofaRow) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    insert_if_present(&mut fields, "host", row.host.as_ref());
    insert_if_present(&mut fields, "ip", row.ip.as_ref());
    insert_if_present(&mut fields, "port", row.port.as_ref());
    insert_if_present(&mut fields, "protocol", row.protocol.as_ref());
    insert_if_present(&mut fields, "title", row.title.as_ref());
    insert_if_present(&mut fields, "server", row.server.as_ref());
    insert_if_present(&mut fields, "country", row.country.as_ref());
    fields
}

/// `Site` (网络资产) mapping — host/ip/port + service fingerprint.
pub fn map_site(row: FofaRow, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = base_fields(&row);
    // Surface domain when present so organizations.domains can be filled
    // even when the user searches by "host" or "ip".
    insert_if_present(&mut fields, "domain", row.domain.as_ref());
    if let Some(cert) = &row.cert {
        if !cert.is_empty() {
            fields.insert("cert".into(), cert.clone());
        }
    }
    ProviderRecord::new(PROVIDER, QueryType::Site, fields, raw)
}

/// `Domain` (子域名) mapping — emit `domain` key so the writer routes it
/// into `organizations.domains`.
pub fn map_domain(row: FofaRow, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = base_fields(&row);
    // Prefer the explicit `domain` column; fall back to `host`.
    let domain = row.domain.clone().or_else(|| row.host.clone());
    insert_if_present(&mut fields, "domain", domain.as_ref());
    ProviderRecord::new(PROVIDER, QueryType::Domain, fields, raw)
}

/// `Cert` (证书) mapping — surface the certificate subject string into
/// `organizations.certificates`.
pub fn map_cert(row: FofaRow, raw: serde_json::Value) -> ProviderRecord {
    let mut fields = base_fields(&row);
    insert_if_present(&mut fields, "cert", row.cert.as_ref());
    // domain is often present in cert results too — keep it.
    insert_if_present(&mut fields, "domain", row.domain.as_ref());
    ProviderRecord::new(PROVIDER, QueryType::Cert, fields, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_object() -> serde_json::Value {
        serde_json::json!({})
    }

    fn sample_row() -> FofaRow {
        FofaRow {
            host: Some("https://example.com".into()),
            ip: Some("93.184.216.34".into()),
            port: Some("443".into()),
            protocol: Some("https".into()),
            domain: Some("example.com".into()),
            title: Some("Example Domain".into()),
            server: Some("ECS".into()),
            country: Some("US".into()),
            cert: Some("CN=example.com".into()),
        }
    }

    #[test]
    fn site_mapper_extracts_full_row() {
        let rec = map_site(sample_row(), raw_object());
        assert_eq!(rec.provider, "fofa");
        assert_eq!(rec.query_type, QueryType::Site);
        assert_eq!(rec.fields.get("ip").unwrap(), "93.184.216.34");
        assert_eq!(rec.fields.get("port").unwrap(), "443");
        assert_eq!(rec.fields.get("title").unwrap(), "Example Domain");
        assert_eq!(rec.fields.get("cert").unwrap(), "CN=example.com");
        assert_eq!(rec.fields.get("domain").unwrap(), "example.com");
    }

    #[test]
    fn domain_mapper_prefers_domain_over_host() {
        let row = FofaRow {
            host: Some("https://other.example.com".into()),
            domain: Some("example.com".into()),
            ..Default::default()
        };
        let rec = map_domain(row, raw_object());
        assert_eq!(rec.fields.get("domain").unwrap(), "example.com");
    }

    #[test]
    fn domain_mapper_falls_back_to_host_when_domain_missing() {
        let row = FofaRow {
            host: Some("sub.example.com".into()),
            domain: None,
            ..Default::default()
        };
        let rec = map_domain(row, raw_object());
        assert_eq!(rec.fields.get("domain").unwrap(), "sub.example.com");
    }

    #[test]
    fn cert_mapper_surfaces_cert_field() {
        let row = FofaRow {
            cert: Some("CN=*.example.com, O=ExampleOrg".into()),
            domain: Some("example.com".into()),
            ..Default::default()
        };
        let rec = map_cert(row, raw_object());
        assert_eq!(rec.query_type, QueryType::Cert);
        assert!(rec.fields.get("cert").unwrap().contains("CN=*.example.com"));
        assert_eq!(rec.fields.get("domain").unwrap(), "example.com");
    }

    #[test]
    fn empty_row_yields_empty_fields() {
        let row = FofaRow::default();
        let rec = map_site(row, raw_object());
        assert!(rec.fields.is_empty());
    }
}
