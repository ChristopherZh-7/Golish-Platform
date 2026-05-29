//! HTTP client for the FOFA API.
//!
//! Endpoints used:
//! - `GET /api/v1/search/all`  — search by base64-encoded query
//! - `GET /api/v1/info/my`     — get account / quota info (cheap auth probe)
//!
//! Authentication: FOFA v1 requires the *email* AND the *key* together.
//! Because the upstream key store only holds a single string, callers pass
//! the key in the canonical form `"<email>|<key>"`. [`split_credentials`]
//! parses it; missing / malformed combos yield [`IntelError::InvalidKey`].

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Deserialize;
use url::Url;

use crate::error::{IntelError, IntelResult};

pub(crate) const PROVIDER_ID: &str = "fofa";
const SEARCH_URL: &str = "https://fofa.info/api/v1/search/all";
const INFO_URL: &str = "https://fofa.info/api/v1/info/my";
const DEFAULT_SIZE: u32 = 100;

/// Split the canonical `"<email>|<key>"` credential string.
///
/// Both halves must be non-empty after trimming. Surfaces a typed error
/// so callers can prompt the user to re-enter the credential.
pub(crate) fn split_credentials(combined: &str) -> IntelResult<(&str, &str)> {
    let combined = combined.trim();
    let (email, key) = combined
        .split_once('|')
        .ok_or_else(|| IntelError::InvalidKey {
            provider: PROVIDER_ID.into(),
            reason: "expected 'email|key' format".into(),
        })?;
    let email = email.trim();
    let key = key.trim();
    if email.is_empty() || key.is_empty() {
        return Err(IntelError::InvalidKey {
            provider: PROVIDER_ID.into(),
            reason: "email or key portion is empty".into(),
        });
    }
    Ok((email, key))
}

/// Default reqwest client used by [`FofaProvider`]; exposed for tests that
/// want to swap in a mocked transport.
pub fn default_http_client() -> reqwest::Client {
    crate::shared::http_common::default_client()
}

/// Encode the FOFA query string as standard base64.
///
/// FOFA expects standard (not URL-safe) base64 in the `qbase64` parameter.
pub(crate) fn encode_query(q: &str) -> String {
    STANDARD.encode(q)
}

/// Issue a search request and decode the result.
pub(crate) async fn search<T>(
    client: &reqwest::Client,
    email: &str,
    key: &str,
    query: &str,
    fields: &str,
    size: Option<u32>,
) -> IntelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let qbase64 = encode_query(query);
    let size_str = size.unwrap_or(DEFAULT_SIZE).to_string();
    let url = Url::parse_with_params(
        SEARCH_URL,
        &[
            ("email", email),
            ("key", key),
            ("qbase64", qbase64.as_str()),
            ("fields", fields),
            ("size", size_str.as_str()),
            ("page", "1"),
            // `full=false` keeps the query in the recent-1-year window.
            ("full", "false"),
        ],
    )
    .map_err(|e| IntelError::bad_response(PROVIDER_ID, format!("URL build failed: {e}")))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| IntelError::network(PROVIDER_ID, e))?;

    crate::shared::http_common::decode_json_envelope(PROVIDER_ID, resp).await
}

/// Issue the cheap `info/my` request and decode the user-info envelope.
pub(crate) async fn fetch_info<T>(
    client: &reqwest::Client,
    email: &str,
    key: &str,
) -> IntelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let url = Url::parse_with_params(INFO_URL, &[("email", email), ("key", key)])
        .map_err(|e| IntelError::bad_response(PROVIDER_ID, format!("URL build failed: {e}")))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| IntelError::network(PROVIDER_ID, e))?;
    crate::shared::http_common::decode_json_envelope(PROVIDER_ID, resp).await
}

/// Translate a FOFA `errmsg` string into a typed error.
///
/// FOFA's documented error codes (partial):
/// - `[820000] params email or key error` → auth failed
/// - `[820001] account locked` → auth failed
/// - `[820004] f-point exhausted` → quota
/// - `[820006] no permission` → auth (paid feature)
/// - everything else → bad response
pub(crate) fn classify_errmsg(errmsg: &str) -> IntelError {
    let lower = errmsg.to_lowercase();
    if lower.contains("820000")
        || lower.contains("820001")
        || lower.contains("820006")
        || lower.contains("email or key")
        || lower.contains("invalid")
        || lower.contains("expire")
        || lower.contains("permission")
    {
        IntelError::AuthFailed {
            provider: PROVIDER_ID.into(),
            reason: errmsg.to_string(),
        }
    } else if lower.contains("820004")
        || lower.contains("f-point")
        || lower.contains("fpoint")
        || lower.contains("quota")
        || lower.contains("\u{914d}\u{989d}")
        || lower.contains("\u{6b21}\u{6570}")
    {
        IntelError::QuotaExceeded {
            provider: PROVIDER_ID.into(),
            reason: errmsg.to_string(),
        }
    } else {
        IntelError::BadResponse {
            provider: PROVIDER_ID.into(),
            reason: errmsg.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_query_uses_standard_base64() {
        let encoded = encode_query("domain=\"example.com\"");
        assert_eq!(encoded, "ZG9tYWluPSJleGFtcGxlLmNvbSI=");
    }

    #[test]
    fn split_credentials_happy_path() {
        let (email, key) = split_credentials("alice@example.com|abcdef0123456789").unwrap();
        assert_eq!(email, "alice@example.com");
        assert_eq!(key, "abcdef0123456789");
    }

    #[test]
    fn split_credentials_trims_whitespace() {
        let (email, key) = split_credentials("  alice@x.com  |  abc123  ").unwrap();
        assert_eq!(email, "alice@x.com");
        assert_eq!(key, "abc123");
    }

    #[test]
    fn split_credentials_missing_pipe_errors() {
        let err = split_credentials("abcdef0123456789").unwrap_err();
        assert!(matches!(err, IntelError::InvalidKey { .. }));
    }

    #[test]
    fn split_credentials_empty_half_errors() {
        let err = split_credentials("|onlykey").unwrap_err();
        assert!(matches!(err, IntelError::InvalidKey { .. }));
        let err = split_credentials("onlyemail@x.com|").unwrap_err();
        assert!(matches!(err, IntelError::InvalidKey { .. }));
    }

    #[test]
    fn classify_recognizes_auth_codes() {
        let err = classify_errmsg("[820000] params email or key error");
        assert!(matches!(err, IntelError::AuthFailed { .. }));
        let err = classify_errmsg("[820001] account is locked");
        assert!(matches!(err, IntelError::AuthFailed { .. }));
    }

    #[test]
    fn classify_recognizes_quota_codes() {
        let err = classify_errmsg("[820004] f-point exhausted");
        assert!(matches!(err, IntelError::QuotaExceeded { .. }));
    }

    #[test]
    fn classify_falls_back_to_bad_response() {
        let err = classify_errmsg("syntax error near 'xx'");
        assert!(matches!(err, IntelError::BadResponse { .. }));
    }

    #[test]
    fn default_client_builds() {
        let _c = default_http_client();
    }
}
