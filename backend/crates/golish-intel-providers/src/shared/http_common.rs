//! Shared HTTP helpers for provider clients.
//!
//! Centralizes the two things every provider client used to copy verbatim:
//! - [`default_client`] · the reqwest client builder (30s timeout + crate UA)
//! - [`decode_json_envelope`] · the "simple" response decoder
//!   (HTTP 401/403 → `AuthFailed`, other non-2xx → `BadResponse`, else parse
//!   the body as JSON).
//!
//! Providers whose error envelope genuinely differs keep their own decode
//! path instead of calling [`decode_json_envelope`]:
//! - `shodan` parses a `{"error": "..."}` body (incl. on HTTP 200) via
//!   `classify_message`.
//! - `hunter` additionally maps HTTP 429 to `QuotaExceeded`.

use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::error::{IntelError, IntelResult};

const USER_AGENT: &str = concat!("golish-intel-providers/", env!("CARGO_PKG_VERSION"));
const TIMEOUT_SECS: u64 = 30;

/// Build the default reqwest client shared by every provider: a 30-second
/// timeout and the crate's User-Agent. Only panics if reqwest cannot build a
/// client from these static, always-valid defaults.
pub(crate) fn default_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .user_agent(USER_AGENT)
        .build()
        .expect("reqwest client must build with valid defaults")
}

/// Decode a response that follows the common envelope contract:
/// - HTTP 401 / 403 → [`IntelError::AuthFailed`]
/// - any other non-2xx → [`IntelError::bad_response`]
/// - 2xx → parse the body as JSON into `T`
///
/// `provider` is the stable provider id used in the returned errors.
pub(crate) async fn decode_json_envelope<T>(
    provider: &str,
    resp: reqwest::Response,
) -> IntelResult<T>
where
    T: DeserializeOwned,
{
    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(IntelError::AuthFailed {
                provider: provider.into(),
                reason: format!("HTTP {status}"),
            });
        }
        return Err(IntelError::bad_response(provider, format!("HTTP {status}")));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| IntelError::network(provider, e))?;
    serde_json::from_str::<T>(&body).map_err(|e| {
        IntelError::bad_response(
            provider,
            format!(
                "JSON parse failed: {e} · body head: {}",
                &body[..body.len().min(200)]
            ),
        )
    })
}
