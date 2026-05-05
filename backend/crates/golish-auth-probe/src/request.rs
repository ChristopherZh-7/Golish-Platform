//! HTTP request execution — single-round helper used by the orchestrator.

use std::time::Duration;

use anyhow::Result;
use reqwest::Client;

use crate::types::{Round, RoundOutcome, TokenSource};

/// Build a [`reqwest::Client`] tuned for probe usage:
/// - explicit timeout
/// - accept invalid TLS certs (target may be a self-signed staging env)
/// - browser-like user-agent (overridable)
/// - up to 5 redirects
pub(crate) fn build_client(timeout_ms: u64, user_agent: Option<&str>) -> Result<Client> {
    let ua = user_agent.unwrap_or(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
         (KHTML, like Gecko) golish-auth-probe/0.2",
    );
    Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .danger_accept_invalid_certs(true)
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(ua)
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build reqwest client: {}", e))
}

/// Send one request and turn its response (or failure) into a [`Round`].
///
/// `token` is injected as `Authorization: Bearer ...` when [`TokenSource::Plain`]
/// is supplied. We don't try to detect Cookie / X-Token auth here — that's
/// the caller's job (it has the original `Endpoint.auth` to dispatch on).
pub(crate) async fn execute_round(
    client: &Client,
    method: &str,
    url: &str,
    token: &TokenSource,
) -> Round {
    let method_parsed = reqwest::Method::from_bytes(method.as_bytes())
        .unwrap_or(reqwest::Method::GET);
    let mut req = client.request(method_parsed, url);
    if let TokenSource::Plain { value } = token {
        req = req.header("Authorization", format!("Bearer {}", value));
    }
    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u32>().ok());
            let body = resp.bytes().await.ok();
            let (body_len, snippet) = match &body {
                Some(b) => (b.len(), {
                    let take = b.len().min(200);
                    String::from_utf8_lossy(&b[..take]).into_owned()
                }),
                None => (0, String::new()),
            };
            let outcome = classify_status(status);
            Round {
                status,
                body_len,
                snippet,
                outcome,
                retry_after_secs: retry_after,
            }
        }
        Err(e) => {
            tracing::debug!("[auth_probe] request failed for {}: {}", url, e);
            Round {
                status: 0,
                body_len: 0,
                snippet: String::new(),
                outcome: RoundOutcome::NetworkError,
                retry_after_secs: None,
            }
        }
    }
}

fn classify_status(status: u16) -> RoundOutcome {
    match status {
        200..=299 => RoundOutcome::Success,
        401 | 403 => RoundOutcome::AuthDenied,
        404 => RoundOutcome::NotFound,
        429 => RoundOutcome::RateLimited,
        500..=599 => RoundOutcome::ServerError,
        _ => RoundOutcome::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_status_buckets() {
        assert_eq!(classify_status(200), RoundOutcome::Success);
        assert_eq!(classify_status(201), RoundOutcome::Success);
        assert_eq!(classify_status(401), RoundOutcome::AuthDenied);
        assert_eq!(classify_status(403), RoundOutcome::AuthDenied);
        assert_eq!(classify_status(404), RoundOutcome::NotFound);
        assert_eq!(classify_status(429), RoundOutcome::RateLimited);
        assert_eq!(classify_status(500), RoundOutcome::ServerError);
        assert_eq!(classify_status(503), RoundOutcome::ServerError);
        assert_eq!(classify_status(301), RoundOutcome::Other);
    }
}
