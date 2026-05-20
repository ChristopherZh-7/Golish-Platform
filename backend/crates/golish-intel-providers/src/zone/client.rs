//! HTTP client for the 0.zone API.

use std::time::Duration;

use serde::Deserialize;

use crate::error::{IntelError, IntelResult};

const ENDPOINT: &str = "https://0.zone/api/data/";
const USER_AGENT: &str = concat!("golish-intel-providers/", env!("CARGO_PKG_VERSION"));
const TIMEOUT_SECS: u64 = 30;
const DEFAULT_PAGESIZE: u32 = 40;

/// Issue a single POST to `https://0.zone/api/data/`.
///
/// `key` is the user's `zone_key_id`. On success returns the parsed envelope.
/// On HTTP-level failures returns `IntelError::Network`.
///
/// **Auth handling**: 0.zone returns HTTP 200 with `code != 0` for bad keys;
/// we surface that as [`IntelError::AuthFailed`] / [`IntelError::QuotaExceeded`]
/// depending on the message.
pub async fn post_query<T>(
    client: &reqwest::Client,
    query: &str,
    query_type: &str,
    page: u32,
    key: &str,
) -> IntelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let form = [
        ("query", query.to_string()),
        ("query_type", query_type.to_string()),
        ("page", page.to_string()),
        ("pagesize", DEFAULT_PAGESIZE.to_string()),
        ("zone_key_id", key.to_string()),
    ];
    let body = serde_urlencoded::to_string(&form)
        .map_err(|e| IntelError::bad_response("0.zone", format!("form encode failed: {e}")))?;

    let resp = client
        .post(ENDPOINT)
        .header("User-Agent", USER_AGENT)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| IntelError::network("0.zone", e))?;

    let status = resp.status();
    if !status.is_success() {
        // Non-2xx is unusual for 0.zone (it returns 200 even on auth errors).
        // Map 401/403 explicitly; everything else becomes a bad-response error.
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(IntelError::AuthFailed {
                provider: "0.zone".into(),
                reason: format!("HTTP {status}"),
            });
        }
        return Err(IntelError::bad_response("0.zone", format!("HTTP {status}")));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| IntelError::network("0.zone", e))?;

    serde_json::from_str::<T>(&body).map_err(|e| {
        IntelError::bad_response(
            "0.zone",
            format!(
                "JSON parse failed: {e} · body head: {}",
                &body[..body.len().min(200)]
            ),
        )
    })
}

/// Construct a default reqwest client with sensible defaults.
pub fn default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .user_agent(USER_AGENT)
        .build()
        .expect("reqwest client must build with valid defaults")
}

/// Classify a `code != 0` envelope into a typed error.
pub fn classify_envelope_error(message: &str) -> IntelError {
    let lower = message.to_lowercase();
    if lower.contains("key") || lower.contains("token") || lower.contains("auth") {
        IntelError::AuthFailed {
            provider: "0.zone".into(),
            reason: message.to_string(),
        }
    } else if lower.contains("quota")
        || lower.contains("limit")
        || lower.contains("\u{8d85}\u{989d}")
        || lower.contains("\u{6b21}\u{6570}")
    {
        // Chinese "超额" / "次数" for quota / count exhaustion
        IntelError::QuotaExceeded {
            provider: "0.zone".into(),
            reason: message.to_string(),
        }
    } else {
        IntelError::BadResponse {
            provider: "0.zone".into(),
            reason: message.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_recognizes_auth_failure() {
        let err = classify_envelope_error("API key invalid");
        assert!(matches!(err, IntelError::AuthFailed { .. }));
    }

    #[test]
    fn classify_recognizes_quota_chinese() {
        let err = classify_envelope_error(
            "\u{67e5}\u{8be2}\u{6b21}\u{6570}\u{8d85}\u{8fc7}\u{4e0a}\u{9650}",
        );
        // 查询次数超过上限
        assert!(matches!(err, IntelError::QuotaExceeded { .. }));
    }

    #[test]
    fn classify_recognizes_quota_english() {
        let err = classify_envelope_error("Quota exceeded for today");
        assert!(matches!(err, IntelError::QuotaExceeded { .. }));
    }

    #[test]
    fn classify_falls_back_to_bad_response() {
        let err = classify_envelope_error("unexpected server error");
        assert!(matches!(err, IntelError::BadResponse { .. }));
    }

    #[test]
    fn default_client_builds() {
        let _c = default_http_client();
    }
}
