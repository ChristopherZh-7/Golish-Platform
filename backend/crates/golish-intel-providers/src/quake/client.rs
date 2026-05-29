//! HTTP client for the 360 Quake v3 API.
//!
//! Endpoints used:
//! - `POST /api/v3/search/quake_service` (real-time search)
//! - `GET  /api/v3/user/info`            (quota probe)
//!
//! Auth: HTTP header `X-QuakeToken: <api_key>`.
//!
//! Reference: <https://quake.360.net/quake/#/help>

use serde::{Deserialize, Serialize};

use crate::error::{IntelError, IntelResult};

pub(crate) const PROVIDER_ID: &str = "quake";
const SEARCH_URL: &str = "https://quake.360.net/api/v3/search/quake_service";
const USER_INFO_URL: &str = "https://quake.360.net/api/v3/user/info";
const DEFAULT_SIZE: u32 = 100;
const AUTH_HEADER: &str = "X-QuakeToken";

/// Request body for `quake_service` search.
///
/// Only the fields we actually send are listed here. Quake accepts plenty
/// more (`include` / `exclude` / `start_time` / `end_time` / `ignore_cache`),
/// but the default behavior gives results within the recent two weeks which
/// is fine for ASM use cases.
#[derive(Debug, Serialize)]
pub(crate) struct QuakeSearchRequest<'a> {
    pub query: &'a str,
    pub start: u32,
    pub size: u32,
}

/// Default reqwest client; tests can pass in a mocked one via
/// [`super::QuakeProvider::with_http_client`].
pub fn default_http_client() -> reqwest::Client {
    crate::shared::http_common::default_client()
}

/// POST to `/api/v3/search/quake_service`.
pub(crate) async fn search<T>(
    client: &reqwest::Client,
    key: &str,
    query: &str,
    size: Option<u32>,
) -> IntelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let body = QuakeSearchRequest {
        query,
        start: 0,
        size: size.unwrap_or(DEFAULT_SIZE),
    };
    let resp = client
        .post(SEARCH_URL)
        .header(AUTH_HEADER, key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| IntelError::network(PROVIDER_ID, e))?;
    crate::shared::http_common::decode_json_envelope(PROVIDER_ID, resp).await
}

/// GET `/api/v3/user/info`.
pub(crate) async fn fetch_user_info<T>(client: &reqwest::Client, key: &str) -> IntelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let resp = client
        .get(USER_INFO_URL)
        .header(AUTH_HEADER, key)
        .send()
        .await
        .map_err(|e| IntelError::network(PROVIDER_ID, e))?;
    crate::shared::http_common::decode_json_envelope(PROVIDER_ID, resp).await
}

/// Map a non-zero Quake `code` + `message` into a typed error.
///
/// Quake's documented codes (partial):
/// - `q3000` / 401 — invalid token
/// - `q3005` — token does not have permission for this query
/// - `q3015` — query has been blocked
/// - non-numeric "Insufficient credits" → quota
pub(crate) fn classify_envelope(code: i32, message: &str) -> IntelError {
    let lower = message.to_lowercase();
    // Treat explicit auth codes first.
    if code == 401
        || code == 403
        || lower.contains("token")
        || lower.contains("auth")
        || lower.contains("permission")
        || lower.contains("invalid")
    {
        return IntelError::AuthFailed {
            provider: PROVIDER_ID.into(),
            reason: message.to_string(),
        };
    }
    if lower.contains("credit")
        || lower.contains("quota")
        || lower.contains("\u{914d}\u{989d}")
        || lower.contains("\u{6b21}\u{6570}")
        || lower.contains("insufficient")
    {
        return IntelError::QuotaExceeded {
            provider: PROVIDER_ID.into(),
            reason: message.to_string(),
        };
    }
    IntelError::BadResponse {
        provider: PROVIDER_ID.into(),
        reason: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_recognizes_token_failure() {
        let err = classify_envelope(401, "Invalid X-QuakeToken");
        assert!(matches!(err, IntelError::AuthFailed { .. }));
    }

    #[test]
    fn classify_recognizes_credit_exhausted_english() {
        let err = classify_envelope(503, "Insufficient credits to complete this request");
        assert!(matches!(err, IntelError::QuotaExceeded { .. }));
    }

    #[test]
    fn classify_recognizes_quota_chinese() {
        // 配额不足
        let err = classify_envelope(503, "\u{67e5}\u{8be2}\u{914d}\u{989d}\u{4e0d}\u{8db3}");
        assert!(matches!(err, IntelError::QuotaExceeded { .. }));
    }

    #[test]
    fn classify_falls_back_to_bad_response() {
        let err = classify_envelope(500, "Internal server error");
        assert!(matches!(err, IntelError::BadResponse { .. }));
    }

    #[test]
    fn default_client_builds() {
        let _c = default_http_client();
    }
}
