//! HTTP client for the 奇安信 Hunter API.
//!
//! Endpoint: `GET https://hunter.qianxin.com/openApi/search`
//! Docs:     <https://hunter.qianxin.com/home/helpCenter>
//!
//! Auth: `api-key` query string parameter (single token, no email).
//! Query: base64-**URL-safe** encoded (NOT standard base64).

use base64::{engine::general_purpose::URL_SAFE, Engine as _};
use serde::Deserialize;
use url::Url;

use crate::error::{IntelError, IntelResult};

pub(crate) const PROVIDER_ID: &str = "hunter";
const SEARCH_URL: &str = "https://hunter.qianxin.com/openApi/search";
const DEFAULT_PAGE_SIZE: u32 = 100;

/// Default reqwest client tuned for Hunter.
pub fn default_http_client() -> reqwest::Client {
    crate::shared::http_common::default_client()
}

/// URL-safe base64 encode the Hunter query string.
pub(crate) fn encode_query(q: &str) -> String {
    URL_SAFE.encode(q)
}

/// Issue a search request and decode the typed envelope.
pub(crate) async fn search<T>(
    client: &reqwest::Client,
    api_key: &str,
    query: &str,
    is_web: u32,
    page_size: Option<u32>,
) -> IntelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let search_b64 = encode_query(query);
    let page_size_str = page_size.unwrap_or(DEFAULT_PAGE_SIZE).to_string();
    let is_web_str = is_web.to_string();
    let url = Url::parse_with_params(
        SEARCH_URL,
        &[
            ("api-key", api_key),
            ("search", search_b64.as_str()),
            ("page", "1"),
            ("page_size", page_size_str.as_str()),
            ("is_web", is_web_str.as_str()),
        ],
    )
    .map_err(|e| IntelError::bad_response(PROVIDER_ID, format!("URL build failed: {e}")))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| IntelError::network(PROVIDER_ID, e))?;

    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(IntelError::AuthFailed {
                provider: PROVIDER_ID.into(),
                reason: format!("HTTP {status}"),
            });
        }
        if status.as_u16() == 429 {
            return Err(IntelError::QuotaExceeded {
                provider: PROVIDER_ID.into(),
                reason: format!("HTTP {status}"),
            });
        }
        return Err(IntelError::bad_response(
            PROVIDER_ID,
            format!("HTTP {status}"),
        ));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| IntelError::network(PROVIDER_ID, e))?;
    serde_json::from_str::<T>(&body).map_err(|e| {
        IntelError::bad_response(
            PROVIDER_ID,
            format!(
                "JSON parse failed: {e} · body head: {}",
                &body[..body.len().min(200)]
            ),
        )
    })
}

/// Hunter returns `code` field in JSON envelope:
/// - 200 → success
/// - 400/401 → auth or syntax
/// - 401 / 402 → no key / quota out
///
/// We surface those as typed errors.
pub(crate) fn classify_code_error(code: i32, msg: &str) -> IntelError {
    let lower = msg.to_lowercase();
    if code == 401 || lower.contains("key") || lower.contains("auth") {
        IntelError::AuthFailed {
            provider: PROVIDER_ID.into(),
            reason: format!("[{code}] {msg}"),
        }
    } else if lower.contains("quota")
        || lower.contains("limit")
        || lower.contains("\u{6b21}\u{6570}")
        || lower.contains("\u{79ef}\u{5206}")
    {
        IntelError::QuotaExceeded {
            provider: PROVIDER_ID.into(),
            reason: format!("[{code}] {msg}"),
        }
    } else {
        IntelError::BadResponse {
            provider: PROVIDER_ID.into(),
            reason: format!("[{code}] {msg}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_safe_encoding_no_plus_or_slash() {
        // URL-safe base64 should never emit '+' or '/'.
        let encoded = encode_query("ip=\"1.1.1.1\" || domain=\"x.com\"");
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
    }

    #[test]
    fn classify_auth_error() {
        let err = classify_code_error(401, "Unauthorized");
        assert!(matches!(err, IntelError::AuthFailed { .. }));
    }

    #[test]
    fn classify_quota_chinese() {
        let err = classify_code_error(40004, "\u{67e5}\u{8be2}\u{6b21}\u{6570}\u{8d85}\u{9650}");
        assert!(matches!(err, IntelError::QuotaExceeded { .. }));
    }

    #[test]
    fn classify_other_falls_back_to_bad_response() {
        let err = classify_code_error(500, "internal");
        assert!(matches!(err, IntelError::BadResponse { .. }));
    }

    #[test]
    fn default_client_builds() {
        let _c = default_http_client();
    }
}
