//! Wire-format response structures for the 360 Quake v3 API.
//!
//! Endpoints used:
//! - `POST /api/v3/search/quake_service`  (real-time service search)
//! - `GET  /api/v3/user/info`             (quota probe for test_connection)
//!
//! Common envelope:
//! ```json
//! { "code": 0, "message": "Successful.", "data": [...], "meta": {...} }
//! ```
//! where `code == 0` means success and any non-zero value indicates an
//! error (the `message` carries the human-readable reason).
//!
//! Reference: <https://quake.360.net/quake/#/help>
//!
//! NOTE on `#![allow(dead_code)]`: many envelope / pagination fields are
//! parsed defensively from the wire (so non-numeric quotas / unknown server
//! versions don't fail deserialization), but only a subset is currently
//! consumed by the mapper. Future iterations may surface pagination /
//! province / TLS handshake — keeping them here avoids API-shape churn.

#![allow(dead_code)]

use serde::Deserialize;

/// Common envelope shared by both search and user-info responses.
#[derive(Debug, Deserialize, Default)]
pub struct QuakeEnvelope<T> {
    #[serde(default)]
    pub code: i32,
    #[serde(default)]
    pub message: String,
    /// Optional because user_info wraps a single object, search wraps a Vec.
    #[serde(default)]
    pub data: Option<T>,
    #[serde(default)]
    pub meta: Option<QuakeMeta>,
}

impl<T> QuakeEnvelope<T> {
    pub fn is_ok(&self) -> bool {
        self.code == 0
    }
}

/// `meta` block carrying pagination + total count.
#[derive(Debug, Deserialize, Default)]
pub struct QuakeMeta {
    #[serde(default)]
    pub total: Option<QuakeTotal>,
    #[serde(default)]
    pub pagination: Option<QuakePagination>,
}

#[derive(Debug, Deserialize, Default)]
pub struct QuakeTotal {
    #[serde(default)]
    pub value: u64,
    #[serde(default)]
    pub relation: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct QuakePagination {
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub page_index: u64,
    #[serde(default)]
    pub page_size: u64,
    #[serde(default)]
    pub total: u64,
}

/// Single service / host hit from `quake_service`.
///
/// Only fields we actually consume are listed; remaining JSON is preserved
/// in the [`super::super::types::ProviderRecord::raw`] field for evidence.
#[derive(Debug, Deserialize, Default)]
pub struct QuakeService {
    #[serde(default)]
    pub ip: Option<String>,
    #[serde(default)]
    pub port: Option<u32>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub transport: Option<String>,
    /// ASN may come as either a number or a string depending on Quake version.
    #[serde(default)]
    pub asn: Option<serde_json::Value>,
    #[serde(default)]
    pub org: Option<String>,
    /// Nested location block.
    #[serde(default)]
    pub location: Option<QuakeLocation>,
    /// Nested service block (carries service name + protocol-specific data).
    #[serde(default)]
    pub service: Option<QuakeInnerService>,
    /// Certificate subject / fingerprint (Quake sometimes flattens it here).
    #[serde(default)]
    pub cert: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct QuakeLocation {
    #[serde(default)]
    pub country_cn: Option<String>,
    #[serde(default)]
    pub country_en: Option<String>,
    #[serde(default)]
    pub province_cn: Option<String>,
    #[serde(default)]
    pub city_cn: Option<String>,
    #[serde(default)]
    pub isp: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct QuakeInnerService {
    /// Top-level service name (`"http"`, `"ssh"`, `"mysql"`, ...).
    #[serde(default)]
    pub name: Option<String>,
    /// Optional nested HTTP info (title / server header / response).
    #[serde(default)]
    pub http: Option<QuakeHttp>,
    /// TLS / cert info when the service exposes one.
    #[serde(default)]
    pub tls: Option<QuakeTls>,
}

#[derive(Debug, Deserialize, Default)]
pub struct QuakeHttp {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct QuakeTls {
    /// Certificate handshake info — Quake nests deeply; we only keep the
    /// human-readable subject string for the cert field.
    #[serde(default)]
    pub handshake_log: Option<serde_json::Value>,
}

impl QuakeService {
    /// Best-effort ASN extractor (handles both `42` and `"AS42"` shapes).
    pub fn asn_string(&self) -> Option<String> {
        match self.asn.as_ref() {
            None => None,
            Some(serde_json::Value::Number(n)) => Some(format!("AS{n}")),
            Some(serde_json::Value::String(s)) if !s.is_empty() => Some({
                if s.starts_with("AS") {
                    s.clone()
                } else {
                    format!("AS{s}")
                }
            }),
            _ => None,
        }
    }
}

/// User info data block returned by `GET /api/v3/user/info`.
#[derive(Debug, Deserialize, Default)]
pub struct QuakeUserInfo {
    #[serde(default)]
    pub id: Option<String>,
    /// Remaining credit for the current month.
    #[serde(default)]
    pub month_remaining_credit: Option<u64>,
    /// Total monthly credit assigned to this account.
    #[serde(default)]
    pub month_max_credit: Option<u64>,
    /// Daily remaining credit (some plans).
    #[serde(default)]
    pub remaining_credit: Option<u64>,
    /// Optional user object (we only inspect display name for the test_connection message).
    #[serde(default)]
    pub user: Option<QuakeUser>,
}

#[derive(Debug, Deserialize, Default)]
pub struct QuakeUser {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_success_with_search_data_parses() {
        let json = r#"{
            "code": 0,
            "message": "Successful.",
            "data": [{
                "ip": "1.2.3.4",
                "port": 443,
                "domain": "example.com",
                "transport": "tcp",
                "asn": 4134,
                "org": "Cogent",
                "location": {"country_cn": "中国"},
                "service": {"name": "http", "http": {"title": "Hi"}}
            }],
            "meta": {
                "total": {"value": 1, "relation": "eq"},
                "pagination": {"count": 1, "page_index": 0, "page_size": 10, "total": 1}
            }
        }"#;
        let env: QuakeEnvelope<Vec<QuakeService>> = serde_json::from_str(json).unwrap();
        assert!(env.is_ok());
        let data = env.data.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].ip.as_deref(), Some("1.2.3.4"));
        assert_eq!(data[0].port, Some(443));
        assert_eq!(
            data[0].service.as_ref().unwrap().name.as_deref(),
            Some("http")
        );
    }

    #[test]
    fn envelope_error_carries_message() {
        let json =
            r#"{"code": 503, "message": "Invalid X-QuakeToken", "data": null, "meta": null}"#;
        let env: QuakeEnvelope<Vec<QuakeService>> = serde_json::from_str(json).unwrap();
        assert!(!env.is_ok());
        assert_eq!(env.message, "Invalid X-QuakeToken");
    }

    #[test]
    fn asn_as_number_yields_prefixed_string() {
        let s = QuakeService {
            asn: Some(serde_json::json!(4134)),
            ..Default::default()
        };
        assert_eq!(s.asn_string().as_deref(), Some("AS4134"));
    }

    #[test]
    fn asn_as_string_keeps_prefix() {
        let s = QuakeService {
            asn: Some(serde_json::json!("AS4134")),
            ..Default::default()
        };
        assert_eq!(s.asn_string().as_deref(), Some("AS4134"));
    }

    #[test]
    fn asn_as_string_adds_prefix_when_missing() {
        let s = QuakeService {
            asn: Some(serde_json::json!("4134")),
            ..Default::default()
        };
        assert_eq!(s.asn_string().as_deref(), Some("AS4134"));
    }

    #[test]
    fn asn_none_when_missing() {
        let s = QuakeService::default();
        assert!(s.asn_string().is_none());
    }

    #[test]
    fn user_info_envelope_parses() {
        let json = r#"{
            "code": 0,
            "message": "Successful.",
            "data": {
                "id": "user123",
                "month_remaining_credit": 2500,
                "month_max_credit": 3000,
                "user": {"username": "alice", "email": "a@x.com"}
            }
        }"#;
        let env: QuakeEnvelope<QuakeUserInfo> = serde_json::from_str(json).unwrap();
        assert!(env.is_ok());
        let info = env.data.unwrap();
        assert_eq!(info.month_remaining_credit, Some(2500));
        assert_eq!(
            info.user.as_ref().and_then(|u| u.username.as_deref()),
            Some("alice")
        );
    }
}
