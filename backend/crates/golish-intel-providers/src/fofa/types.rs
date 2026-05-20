//! Wire-format response structures for the FOFA API.
//!
//! API: `GET https://fofa.info/api/v1/search/all`
//!
//! FOFA returns a JSON envelope with `error: bool`. On success, `results`
//! is a 2-D array whose columns map to the `fields` parameter sent by the
//! caller — we always request a fixed column set (see [`FOFA_FIELDS`]).
//!
//! On error the envelope sets `error: true` and fills `errmsg`.
//!
//! Reference: <https://fofa.info/api>
//!
//! NOTE on `#![allow(dead_code)]`: envelope metadata fields (size / page /
//! consumed_fpoint / required_fpoints / mode / query) are parsed to keep
//! the deserializer schema-complete but not all are currently surfaced
//! through `ProviderRecord`. Future iterations may emit them as evidence.

#![allow(dead_code)]

use serde::Deserialize;

/// Fixed column ordering we request from `/api/v1/search/all`.
///
/// Keeping this stable matters because `results` is positional, not named.
/// The order here must match [`FofaRow`] field positions.
pub const FOFA_FIELDS: &str = "host,ip,port,protocol,domain,title,server,country,cert";

/// Common envelope shared by success and error responses.
#[derive(Debug, Deserialize, Default)]
pub struct FofaEnvelope {
    /// `true` when the request failed (auth/quota/syntax).
    #[serde(default)]
    pub error: bool,
    /// Error message (populated when `error == true`).
    #[serde(default)]
    pub errmsg: Option<String>,
    /// Total result count (across all pages).
    #[serde(default)]
    pub size: u64,
    /// Current page index (1-based).
    #[serde(default)]
    pub page: u32,
    /// Engine mode (e.g. `"extended"`).
    #[serde(default)]
    pub mode: Option<String>,
    /// Echo of the FOFA query string we sent.
    #[serde(default)]
    pub query: Option<String>,
    /// 2-D array; each inner array is one match, column order = [`FOFA_FIELDS`].
    #[serde(default)]
    pub results: Vec<Vec<String>>,
    /// F-points consumed by this request (FOFA quota currency).
    #[serde(default)]
    pub consumed_fpoint: Option<u64>,
    /// F-points required for the request.
    #[serde(default)]
    pub required_fpoints: Option<u64>,
}

impl FofaEnvelope {
    /// Returns true when the response is a successful query result.
    pub fn is_ok(&self) -> bool {
        !self.error
    }

    /// Convert one positional row into a [`FofaRow`].
    ///
    /// Returns `None` when the row is empty (defensive — FOFA shouldn't
    /// emit empty rows, but we tolerate it).
    pub fn row(row: &[String]) -> Option<FofaRow> {
        if row.is_empty() {
            return None;
        }
        // FOFA pads short rows with empty strings; safe to index defensively.
        let get = |i: usize| row.get(i).cloned().filter(|s| !s.is_empty());
        Some(FofaRow {
            host: get(0),
            ip: get(1),
            port: get(2),
            protocol: get(3),
            domain: get(4),
            title: get(5),
            server: get(6),
            country: get(7),
            cert: get(8),
        })
    }
}

/// Decoded single match row.
///
/// Field order **must** mirror [`FOFA_FIELDS`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FofaRow {
    pub host: Option<String>,
    pub ip: Option<String>,
    pub port: Option<String>,
    pub protocol: Option<String>,
    pub domain: Option<String>,
    pub title: Option<String>,
    pub server: Option<String>,
    pub country: Option<String>,
    pub cert: Option<String>,
}

/// FOFA "User Info" endpoint response (used by `test_connection` to fetch
/// quota figures cheaply).
///
/// Endpoint: `GET https://fofa.info/api/v1/info/my`
#[derive(Debug, Deserialize, Default)]
pub struct FofaUserInfo {
    #[serde(default)]
    pub error: bool,
    #[serde(default)]
    pub errmsg: Option<String>,
    /// User-visible email (sanity check the key belongs to a real account).
    #[serde(default)]
    pub email: Option<String>,
    /// Daily search quota allowed for this account.
    #[serde(default)]
    pub fofa_point: Option<u64>,
    /// Remaining F-points / search quota.
    #[serde(default)]
    pub remain_free_point: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_success_parses() {
        let json = r#"{
            "error": false,
            "consumed_fpoint": 1,
            "required_fpoints": 0,
            "size": 2,
            "page": 1,
            "mode": "extended",
            "query": "domain=\"example.com\"",
            "results": [
                ["example.com", "93.184.216.34", "443", "https", "example.com", "Example Domain", "ECS (sec/97A6)", "US", ""],
                ["www.example.com", "93.184.216.34", "443", "https", "example.com", "Example", "", "US", ""]
            ]
        }"#;
        let env: FofaEnvelope = serde_json::from_str(json).expect("parse");
        assert!(env.is_ok());
        assert_eq!(env.size, 2);
        assert_eq!(env.results.len(), 2);
        assert_eq!(env.consumed_fpoint, Some(1));
    }

    #[test]
    fn envelope_error_parses() {
        let json = r#"{"error": true, "errmsg": "[820000] params email or key error"}"#;
        let env: FofaEnvelope = serde_json::from_str(json).unwrap();
        assert!(!env.is_ok());
        assert_eq!(
            env.errmsg.as_deref(),
            Some("[820000] params email or key error")
        );
        assert!(env.results.is_empty());
    }

    #[test]
    fn envelope_with_missing_optionals_parses() {
        // Partial envelope (rare, but be defensive).
        let json = r#"{"error": false, "size": 0, "page": 1, "results": []}"#;
        let env: FofaEnvelope = serde_json::from_str(json).unwrap();
        assert!(env.is_ok());
        assert!(env.results.is_empty());
        assert!(env.mode.is_none());
    }

    #[test]
    fn row_decoder_handles_full_row() {
        let raw = vec![
            "example.com".into(),
            "1.2.3.4".into(),
            "443".into(),
            "https".into(),
            "example.com".into(),
            "Hello".into(),
            "nginx".into(),
            "US".into(),
            "CN=example.com".into(),
        ];
        let row = FofaEnvelope::row(&raw).unwrap();
        assert_eq!(row.host.as_deref(), Some("example.com"));
        assert_eq!(row.ip.as_deref(), Some("1.2.3.4"));
        assert_eq!(row.port.as_deref(), Some("443"));
        assert_eq!(row.title.as_deref(), Some("Hello"));
        assert_eq!(row.cert.as_deref(), Some("CN=example.com"));
    }

    #[test]
    fn row_decoder_treats_empty_string_as_none() {
        let raw = vec!["host".into(), "".into(), "80".into()];
        let row = FofaEnvelope::row(&raw).unwrap();
        assert_eq!(row.host.as_deref(), Some("host"));
        assert!(row.ip.is_none(), "empty string should decode as None");
        assert_eq!(row.port.as_deref(), Some("80"));
    }

    #[test]
    fn row_decoder_returns_none_for_empty_row() {
        let raw: Vec<String> = vec![];
        assert!(FofaEnvelope::row(&raw).is_none());
    }

    #[test]
    fn user_info_parses() {
        let json = r#"{
            "error": false,
            "email": "tester@example.com",
            "fofa_point": 100,
            "remain_free_point": 80
        }"#;
        let info: FofaUserInfo = serde_json::from_str(json).unwrap();
        assert!(!info.error);
        assert_eq!(info.email.as_deref(), Some("tester@example.com"));
        assert_eq!(info.remain_free_point, Some(80));
    }
}
