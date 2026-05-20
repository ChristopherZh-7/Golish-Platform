//! HTTP client for the Shodan REST API.
//!
//! Endpoints used:
//! - `GET /shodan/host/search?key={k}&query={q}`   (host/cert search · paid)
//! - `GET /dns/domain/{domain}?key={k}`             (subdomain enum · free)
//! - `GET /api-info?key={k}`                        (quota probe)
//!
//! Reference: <https://developer.shodan.io/api>

use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::error::{IntelError, IntelResult};

pub(crate) const PROVIDER_ID: &str = "shodan";
const BASE_URL: &str = "https://api.shodan.io";
const USER_AGENT: &str = concat!("golish-intel-providers/", env!("CARGO_PKG_VERSION"));
const TIMEOUT_SECS: u64 = 30;

/// Default reqwest client; tests can substitute via [`super::ShodanProvider::with_http_client`].
pub fn default_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .user_agent(USER_AGENT)
        .build()
        .expect("reqwest client must build with valid defaults")
}

/// Issue `GET /shodan/host/search?key=...&query=...`.
pub(crate) async fn host_search<T>(
    client: &reqwest::Client,
    key: &str,
    query: &str,
) -> IntelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let url = Url::parse_with_params(
        &format!("{BASE_URL}/shodan/host/search"),
        &[("key", key), ("query", query)],
    )
    .map_err(|e| IntelError::bad_response(PROVIDER_ID, format!("URL build failed: {e}")))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| IntelError::network(PROVIDER_ID, e))?;
    decode_envelope(resp).await
}

/// Issue `GET /dns/domain/{domain}?key=...` (subdomain enumeration).
///
/// Currently unused; kept for the upcoming Domain query_type wiring in
/// `mod.rs::ShodanProvider::query` (see baseline doc §6.4).
#[allow(dead_code)]
pub(crate) async fn dns_domain<T>(
    client: &reqwest::Client,
    key: &str,
    domain: &str,
) -> IntelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = format!(
        "{BASE_URL}/dns/domain/{}",
        urlencoding_encode_path_segment(domain)
    );
    let url = Url::parse_with_params(&raw, &[("key", key)])
        .map_err(|e| IntelError::bad_response(PROVIDER_ID, format!("URL build failed: {e}")))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| IntelError::network(PROVIDER_ID, e))?;
    decode_envelope(resp).await
}

/// Issue `GET /api-info?key=...` for `test_connection`.
pub(crate) async fn api_info<T>(client: &reqwest::Client, key: &str) -> IntelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let url = Url::parse_with_params(&format!("{BASE_URL}/api-info"), &[("key", key)])
        .map_err(|e| IntelError::bad_response(PROVIDER_ID, format!("URL build failed: {e}")))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| IntelError::network(PROVIDER_ID, e))?;
    decode_envelope(resp).await
}

/// Decode the response, surfacing HTTP-level failures as typed errors and
/// Shodan-level `{"error": "..."}` bodies via the caller.
async fn decode_envelope<T>(resp: reqwest::Response) -> IntelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| IntelError::network(PROVIDER_ID, e))?;

    if !status.is_success() {
        if let Ok(env) = serde_json::from_str::<super::types::ShodanErrorEnvelope>(&body) {
            if let Some(msg) = env.error {
                return Err(classify_message(status.as_u16(), &msg));
            }
        }
        return Err(map_http_status(status.as_u16(), &body));
    }

    // Some 200 responses still embed an "error" field; bubble that as auth/quota.
    if let Ok(env) = serde_json::from_str::<super::types::ShodanErrorEnvelope>(&body) {
        if let Some(msg) = env.error.filter(|m| !m.is_empty()) {
            return Err(classify_message(200, &msg));
        }
    }

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

fn map_http_status(status: u16, body: &str) -> IntelError {
    match status {
        401 => IntelError::AuthFailed {
            provider: PROVIDER_ID.into(),
            reason: format!("HTTP 401 · {}", &body[..body.len().min(120)]),
        },
        403 => IntelError::AuthFailed {
            provider: PROVIDER_ID.into(),
            reason: format!("HTTP 403 · {}", &body[..body.len().min(120)]),
        },
        429 => IntelError::QuotaExceeded {
            provider: PROVIDER_ID.into(),
            reason: format!("HTTP 429 · {}", &body[..body.len().min(120)]),
        },
        _ => IntelError::bad_response(PROVIDER_ID, format!("HTTP {status}")),
    }
}

/// Classify a Shodan error message into a typed error.
pub(crate) fn classify_message(http_status: u16, msg: &str) -> IntelError {
    let lower = msg.to_lowercase();
    if http_status == 401
        || lower.contains("api key")
        || lower.contains("invalid")
        || lower.contains("unauthorized")
    {
        return IntelError::AuthFailed {
            provider: PROVIDER_ID.into(),
            reason: msg.to_string(),
        };
    }
    if http_status == 403 || lower.contains("plan") || lower.contains("access") {
        return IntelError::AuthFailed {
            provider: PROVIDER_ID.into(),
            reason: msg.to_string(),
        };
    }
    if http_status == 429
        || lower.contains("rate limit")
        || lower.contains("usage")
        || lower.contains("credit")
        || lower.contains("quota")
    {
        return IntelError::QuotaExceeded {
            provider: PROVIDER_ID.into(),
            reason: msg.to_string(),
        };
    }
    IntelError::BadResponse {
        provider: PROVIDER_ID.into(),
        reason: msg.to_string(),
    }
}

/// Best-effort URL-encode a path segment without bringing in a new crate.
fn urlencoding_encode_path_segment(s: &str) -> String {
    let mut buf = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_') {
            buf.push(ch);
        } else {
            let mut bytes = [0u8; 4];
            for &b in ch.encode_utf8(&mut bytes).as_bytes() {
                buf.push_str(&format!("%{:02X}", b));
            }
        }
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_recognizes_invalid_key() {
        let err = classify_message(401, "Invalid API key");
        assert!(matches!(err, IntelError::AuthFailed { .. }));
    }

    #[test]
    fn classify_recognizes_plan_restriction() {
        let err = classify_message(403, "API plan does not have access to this endpoint");
        assert!(matches!(err, IntelError::AuthFailed { .. }));
    }

    #[test]
    fn classify_recognizes_rate_limit() {
        let err = classify_message(429, "Rate limit exceeded");
        assert!(matches!(err, IntelError::QuotaExceeded { .. }));
        let err = classify_message(200, "Insufficient query credits");
        assert!(matches!(err, IntelError::QuotaExceeded { .. }));
    }

    #[test]
    fn classify_falls_back_to_bad_response() {
        let err = classify_message(500, "Unexpected error");
        assert!(matches!(err, IntelError::BadResponse { .. }));
    }

    #[test]
    fn urlencoding_passes_through_ascii() {
        assert_eq!(
            urlencoding_encode_path_segment("example.com"),
            "example.com"
        );
        assert_eq!(
            urlencoding_encode_path_segment("sub.test-org.io"),
            "sub.test-org.io"
        );
    }

    #[test]
    fn urlencoding_escapes_non_ascii() {
        let s = urlencoding_encode_path_segment("\u{4e2d}.cn");
        assert_eq!(s, "%E4%B8%AD.cn");
    }

    #[test]
    fn http_status_maps_to_typed_errors() {
        assert!(matches!(
            map_http_status(401, ""),
            IntelError::AuthFailed { .. }
        ));
        assert!(matches!(
            map_http_status(403, ""),
            IntelError::AuthFailed { .. }
        ));
        assert!(matches!(
            map_http_status(429, ""),
            IntelError::QuotaExceeded { .. }
        ));
        assert!(matches!(
            map_http_status(500, ""),
            IntelError::BadResponse { .. }
        ));
    }

    #[test]
    fn default_client_builds() {
        let _c = default_http_client();
    }
}
