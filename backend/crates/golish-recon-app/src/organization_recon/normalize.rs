use std::collections::BTreeMap;
use std::net::IpAddr;

use url::Url;

use super::types::{NormalizedReconRecord, ReconRecordKind};

pub(crate) fn normalize_record_key(kind: &ReconRecordKind, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("record value is empty".into());
    }
    match kind {
        ReconRecordKind::Domain => Ok(format!(
            "domain:{}",
            value.trim_end_matches('.').to_ascii_lowercase()
        )),
        ReconRecordKind::Ip => value
            .parse::<IpAddr>()
            .map(|ip| format!("ip:{ip}"))
            .map_err(|error| format!("invalid IP '{value}': {error}")),
        ReconRecordKind::Url | ReconRecordKind::Site => normalize_url(value)
            .map(|url| format!("{}:{url}", record_kind_label(kind)))
            .map_err(|error| format!("invalid URL '{value}': {error}")),
        _ => Ok(format!(
            "{}:{}",
            record_kind_label(kind),
            value.to_lowercase()
        )),
    }
}

pub(crate) fn merge_normalized_records(
    records: impl IntoIterator<Item = NormalizedReconRecord>,
) -> Vec<NormalizedReconRecord> {
    let mut merged: BTreeMap<String, NormalizedReconRecord> = BTreeMap::new();
    for mut record in records {
        match merged.get_mut(&record.key) {
            Some(existing) => {
                for evidence in record.evidence.drain(..) {
                    if !existing.evidence.contains(&evidence) {
                        existing.evidence.push(evidence);
                    }
                }
            }
            None => {
                merged.insert(record.key.clone(), record);
            }
        }
    }
    merged.into_values().collect()
}

fn normalize_url(value: &str) -> Result<String, url::ParseError> {
    let mut url = Url::parse(value)?;
    let normalized_host = url.host_str().map(|host| host.to_ascii_lowercase());
    let _ = url.set_host(normalized_host.as_deref());
    if matches!(
        (url.scheme(), url.port()),
        ("http", Some(80)) | ("https", Some(443))
    ) {
        let _ = url.set_port(None);
    }
    url.set_fragment(None);
    Ok(url.to_string())
}

fn record_kind_label(kind: &ReconRecordKind) -> &'static str {
    match kind {
        ReconRecordKind::Organization => "organization",
        ReconRecordKind::Domain => "domain",
        ReconRecordKind::Ip => "ip",
        ReconRecordKind::Port => "port",
        ReconRecordKind::Service => "service",
        ReconRecordKind::Url => "url",
        ReconRecordKind::Site => "site",
        ReconRecordKind::App => "app",
        ReconRecordKind::MiniProgram => "mini_program",
        ReconRecordKind::Wechat => "wechat",
        ReconRecordKind::Certificate => "certificate",
        ReconRecordKind::Contact => "contact",
        ReconRecordKind::Leak => "leak",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::organization_recon::types::ReconEvidenceRef;

    #[test]
    fn domain_key_lowercases_and_removes_trailing_dot() {
        assert_eq!(
            normalize_record_key(&ReconRecordKind::Domain, "WWW.Example.COM.").unwrap(),
            "domain:www.example.com"
        );
    }

    #[test]
    fn url_key_removes_default_port_and_fragment() {
        assert_eq!(
            normalize_record_key(&ReconRecordKind::Url, "HTTPS://Example.COM:443/a#frag").unwrap(),
            "url:https://example.com/a"
        );
    }

    #[test]
    fn merge_preserves_distinct_evidence() {
        let evidence = |source: &str| ReconEvidenceRef {
            source_id: source.into(),
            run_id: "run".into(),
            task_id: source.into(),
            raw_artifact_path: format!("raw/{source}.json"),
        };
        let record = |source: &str| NormalizedReconRecord {
            record_id: "domain:example.com".into(),
            kind: ReconRecordKind::Domain,
            key: "domain:example.com".into(),
            value: "example.com".into(),
            attributes: Value::Null,
            evidence: vec![evidence(source)],
        };

        let merged = merge_normalized_records([record("enscan"), record("0.zone")]);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].evidence.len(), 2);
    }
}
