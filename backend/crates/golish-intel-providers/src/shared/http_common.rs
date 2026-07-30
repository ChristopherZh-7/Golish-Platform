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

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::error::{IntelError, IntelResult};

const USER_AGENT: &str = concat!("golish-intel-providers/", env!("CARGO_PKG_VERSION"));
const TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Copy)]
struct PublicOnlyDnsResolver;

fn prohibited_provider_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            matches!(
                octets,
                [0, ..]
                    | [10, ..]
                    | [100, 64..=127, ..]
                    | [127, ..]
                    | [169, 254, ..]
                    | [172, 16..=31, ..]
                    | [192, 0, 0, ..]
                    | [192, 0, 2, ..]
                    | [192, 168, ..]
                    | [198, 18..=19, ..]
                    | [198, 51, 100, ..]
                    | [203, 0, 113, ..]
                    | [224..=255, ..]
            )
        }
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return prohibited_provider_ip(IpAddr::V4(mapped));
            }
            let segments = address.segments();
            address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address.is_multicast()
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    }
}

impl reqwest::dns::Resolve for PublicOnlyDnsResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let mut addresses = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .map(|address| address.ip())
                .collect::<Vec<_>>();
            addresses.sort();
            addresses.dedup();
            if addresses.is_empty() || addresses.iter().copied().any(prohibited_provider_ip) {
                return Err(std::io::Error::other("TOOL_TRUTH_DESTINATION_POLICY_BLOCKED").into());
            }
            let addresses = addresses
                .into_iter()
                .map(|address| SocketAddr::new(address, 0))
                .collect::<Vec<_>>();
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Build the default reqwest client shared by every provider: a 30-second
/// timeout and the crate's User-Agent. Only panics if reqwest cannot build a
/// client from these static, always-valid defaults.
pub(crate) fn default_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECS))
        .user_agent(USER_AGENT)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(PublicOnlyDnsResolver))
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

#[cfg(test)]
mod tests {
    use super::prohibited_provider_ip;
    use std::net::IpAddr;

    #[test]
    fn public_only_resolver_rejects_mapped_and_reserved_addresses() {
        for address in [
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "100.64.0.1",
            "198.51.100.2",
            "2001:db8::1",
        ] {
            assert!(prohibited_provider_ip(
                address.parse::<IpAddr>().expect("reserved IP fixture")
            ));
        }
        assert!(!prohibited_provider_ip(
            "1.1.1.1".parse::<IpAddr>().expect("public IP fixture")
        ));
    }
}
