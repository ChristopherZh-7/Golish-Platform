//! ASM intel-provider IPC facade.
//!
//! Bridges the `golish-intel-providers` crate's `IntelProvider` trait into
//! Tauri-callable commands. Provides:
//!
//! - [`intel_list_providers`] — UI lists all known providers + their meta
//! - [`intel_test_connection`] — UI button verifies a configured API key
//! - [`intel_query_provider`] — UI / agent triggers a query, results go
//!   straight into `organizations` via `output_store::store_organization_update`
//!
//! API keys are fetched from the existing `vault_entries` table
//! (entry_type=`api_key`, name=`<provider_id>`, tags=`["intel-provider"]`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
#[cfg(test)]
use std::net::Ipv4Addr;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use golish_intel_providers::shared::KeyStore;
use golish_intel_providers::{
    error::IntelError, fofa::FofaProvider, hunter::HunterProvider, quake::QuakeProvider,
    shodan::ShodanProvider, zone::ZoneProvider, ConnectionStatus, IntelProvider, ProviderMeta,
    ProviderRecord, QueryType,
};
use golish_pentest::config::ControlledFixtureIntelTransportAuthority;
use golish_pentest::output_store::OutputStore;

use golish_app_core::DbState;
use golish_app_core::GolishError;

/// `KeyStore` impl that reads from the `vault_entries` table.
///
/// Lookup convention: vault entry **name** matches the provider id
/// (e.g. `"0.zone"`), entry_type is `api_key`. The newest matching row
/// wins (ORDER BY created_at DESC LIMIT 1).
struct PgVaultKeyStore {
    pool: PgPool,
}

#[async_trait]
impl KeyStore for PgVaultKeyStore {
    async fn get_key(
        &self,
        provider_id: &str,
    ) -> golish_intel_providers::IntelResult<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT value FROM vault_entries \
             WHERE name = $1 AND entry_type = 'api_key' \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(provider_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| IntelError::Other(format!("vault read failed: {e}")))?;

        // NOTE: golish-core's vault stores `value` as base64-obfuscated bytes
        // via `golish_core::vault::deobfuscate`. We call it here so the
        // returned string is the cleartext API key.
        match row {
            None => Ok(None),
            Some((obf,)) => match golish_core::vault::deobfuscate(&obf) {
                Ok(plain) => Ok(Some(plain)),
                Err(e) => Err(IntelError::Other(format!("deobfuscate failed: {e}"))),
            },
        }
    }
}

pub(crate) fn provider_registry() -> HashMap<String, Arc<dyn IntelProvider>> {
    let mut m: HashMap<String, Arc<dyn IntelProvider>> = HashMap::new();
    m.insert("0.zone".into(), Arc::new(ZoneProvider::default()));
    m.insert("fofa".into(), Arc::new(FofaProvider::default()));
    m.insert("quake".into(), Arc::new(QuakeProvider::default()));
    m.insert("hunter".into(), Arc::new(HunterProvider::default()));
    m.insert("shodan".into(), Arc::new(ShodanProvider::default()));
    m
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderExecutionRequest {
    pub(crate) provider_id: String,
    pub(crate) provider_version: String,
    pub(crate) input_key: String,
    pub(crate) target_input: String,
    pub(crate) max_requests: u64,
    pub(crate) exhaustive_empty_contract: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FixedProviderEndpoint {
    scheme: &'static str,
    host: &'static str,
    port: u16,
    path: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIntelFixedProviderEndpoint {
    pub scheme: String,
    pub normalized_host: String,
    pub port: u16,
    pub path_prefix: String,
}

/// Closed endpoint registry used to seal the receipt policy before a passive
/// TargetIntel tool can perform provider I/O. Runtime selection may use a
/// subset, but no target/model input can add a destination.
pub fn target_intel_fixed_provider_endpoints() -> Vec<TargetIntelFixedProviderEndpoint> {
    [
        "0.zone",
        "fofa",
        "quake",
        "hunter",
        "shodan",
        "github-public",
        "rdap",
    ]
    .into_iter()
    .filter_map(fixed_provider_endpoint)
    .map(|endpoint| TargetIntelFixedProviderEndpoint {
        scheme: endpoint.scheme.to_string(),
        normalized_host: endpoint.host.to_string(),
        port: endpoint.port,
        path_prefix: endpoint.path.to_string(),
    })
    .collect()
}

pub fn target_intel_provider_endpoints(
    controlled_fixture: Option<&ControlledFixtureIntelTransportAuthority>,
) -> Vec<TargetIntelFixedProviderEndpoint> {
    if let Some(authority) = controlled_fixture {
        let endpoint = authority.endpoint();
        return vec![TargetIntelFixedProviderEndpoint {
            scheme: endpoint.scheme().to_string(),
            normalized_host: endpoint.host_str().unwrap_or_default().to_string(),
            port: endpoint.port().unwrap_or_default(),
            path_prefix: endpoint.path().to_string(),
        }];
    }
    target_intel_fixed_provider_endpoints()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIntelReceiptSession {
    pub id: Uuid,
    /// Exact response-loss replay of an already finalized current receipt.
    /// When present the tool must return it without performing provider I/O.
    pub replay_result: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct TargetIntelReceiptBegin {
    pub capability: String,
    pub endpoints: Vec<TargetIntelFixedProviderEndpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIntelTechniqueObservation {
    pub technique: String,
    pub observation_state: String,
}

#[derive(Debug, Clone)]
pub struct TargetIntelReceiptFinalization {
    pub provider_evidence: Vec<serde_json::Value>,
    pub technique_observations: Vec<TargetIntelTechniqueObservation>,
    pub typed_landing: serde_json::Value,
    pub failure_reason_code: Option<String>,
}

/// Host-owned canonical receipt side channel. Raw provider artifacts stay out
/// of the model-visible tool result: the composition root injects an app host
/// that begins before I/O and seals the result after business landing.
#[async_trait]
pub trait TargetIntelReceiptObserver: Send + Sync {
    async fn begin(
        &self,
        request: TargetIntelReceiptBegin,
    ) -> anyhow::Result<Option<TargetIntelReceiptSession>>;

    async fn finalize(
        &self,
        session: TargetIntelReceiptSession,
        result: TargetIntelReceiptFinalization,
    ) -> anyhow::Result<()>;
}

fn fixed_provider_endpoint(provider_id: &str) -> Option<FixedProviderEndpoint> {
    let endpoint = match provider_id {
        "0.zone" => ("0.zone", "/api/data/"),
        "fofa" => ("fofa.info", "/api/v1/search/all"),
        "quake" => ("quake.360.net", "/api/v3/search/quake_service"),
        "hunter" => ("hunter.qianxin.com", "/openApi/search"),
        "shodan" => ("api.shodan.io", "/shodan/host/search"),
        "github-public" => ("api.github.com", "/search/repositories"),
        "rdap" => ("rdap.org", "/domain/"),
        #[cfg(test)]
        "fixture" => ("fixed.provider.example.test", "/v1/query"),
        _ => return None,
    };
    Some(FixedProviderEndpoint {
        scheme: "https",
        host: endpoint.0,
        port: 443,
        path: endpoint.1,
    })
}

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

pub(crate) fn validate_fixed_provider_destination(
    provider_id: &str,
    descriptor_url: &str,
    rendered_url: &str,
) -> Result<url::Url, ProviderTransportError> {
    validate_provider_destination(provider_id, descriptor_url, rendered_url, None)
}

pub(crate) fn validate_provider_destination(
    provider_id: &str,
    descriptor_url: &str,
    rendered_url: &str,
    controlled_fixture: Option<&ControlledFixtureIntelTransportAuthority>,
) -> Result<url::Url, ProviderTransportError> {
    let descriptor = url::Url::parse(descriptor_url)
        .map_err(|_| ProviderTransportError::DestinationPolicyBlocked)?;
    let rendered = url::Url::parse(rendered_url)
        .map_err(|_| ProviderTransportError::DestinationPolicyBlocked)?;
    if let Some(authority) =
        controlled_fixture.filter(|authority| provider_id == authority.provider_id())
    {
        let exact_endpoint = |url: &url::Url| {
            let endpoint = authority.endpoint();
            url.scheme() == endpoint.scheme()
                && url.host_str() == endpoint.host_str()
                && url.port() == endpoint.port()
                && url.path() == endpoint.path()
                && url.fragment().is_none()
                && url.username().is_empty()
                && url.password().is_none()
        };
        if exact_endpoint(&descriptor) && exact_endpoint(&rendered) {
            return Ok(rendered);
        }
        return Err(ProviderTransportError::DestinationPolicyBlocked);
    }

    let fixed = fixed_provider_endpoint(provider_id)
        .ok_or(ProviderTransportError::DestinationPolicyBlocked)?;
    let exact_authority = |url: &url::Url| {
        url.scheme() == fixed.scheme
            && url.host_str() == Some(fixed.host)
            && url.port_or_known_default() == Some(fixed.port)
            && if provider_id == "rdap" {
                url.path().starts_with(fixed.path)
            } else {
                url.path() == fixed.path
            }
            && url.username().is_empty()
            && url.password().is_none()
    };
    if !exact_authority(&descriptor) || !exact_authority(&rendered) {
        return Err(ProviderTransportError::DestinationPolicyBlocked);
    }
    Ok(rendered)
}

pub(crate) async fn build_pinned_provider_client(
    endpoint: &url::Url,
    previous_addresses: Option<&[IpAddr]>,
) -> Result<(reqwest::Client, Vec<IpAddr>, IpAddr), ProviderTransportError> {
    build_pinned_provider_client_with_authority(endpoint, previous_addresses, None).await
}

pub(crate) async fn build_pinned_provider_client_with_authority(
    endpoint: &url::Url,
    previous_addresses: Option<&[IpAddr]>,
    controlled_fixture: Option<&ControlledFixtureIntelTransportAuthority>,
) -> Result<(reqwest::Client, Vec<IpAddr>, IpAddr), ProviderTransportError> {
    let host = endpoint
        .host_str()
        .ok_or(ProviderTransportError::DestinationPolicyBlocked)?;
    let port = endpoint
        .port_or_known_default()
        .ok_or(ProviderTransportError::DestinationPolicyBlocked)?;
    let mut addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| ProviderTransportError::Transport(error.to_string()))?
        .map(|address| address.ip())
        .collect::<Vec<_>>();
    addresses.sort();
    addresses.dedup();
    let controlled_address = controlled_fixture
        .filter(|authority| {
            let expected = authority.endpoint();
            endpoint.scheme() == expected.scheme()
                && endpoint.host_str() == expected.host_str()
                && endpoint.port() == expected.port()
                && endpoint.path() == expected.path()
        })
        .and_then(|authority| authority.endpoint().host_str())
        .and_then(|host| host.parse::<IpAddr>().ok());
    let addresses_allowed = match controlled_address {
        Some(expected) => {
            expected.is_loopback() && addresses.iter().all(|address| *address == expected)
        }
        None => !addresses.iter().copied().any(prohibited_provider_ip),
    };
    if addresses.is_empty() || !addresses_allowed {
        return Err(ProviderTransportError::DestinationPolicyBlocked);
    }
    if previous_addresses.is_some_and(|previous| previous != addresses.as_slice()) {
        return Err(ProviderTransportError::DestinationPolicyBlocked);
    }
    let selected_address = addresses[0];
    let socket_addresses = [SocketAddr::new(selected_address, port)];
    let client = reqwest::Client::builder()
        .user_agent(concat!("golish/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &socket_addresses)
        .build()
        .map_err(|error| ProviderTransportError::Transport(error.to_string()))?;
    Ok((client, addresses, selected_address))
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ProviderNetworkHop {
    pub(crate) hop_kind: &'static str,
    pub(crate) normalized_host: String,
    pub(crate) url: String,
    pub(crate) pinned_addresses: Vec<IpAddr>,
    pub(crate) selected_address: IpAddr,
    pub(crate) send_ordinal: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ObservedProviderBudget {
    required_axes: BTreeSet<String>,
    observed_axes: BTreeMap<String, u64>,
}

#[cfg(test)]
impl ObservedProviderBudget {
    pub(crate) fn axis(&self, axis: &str) -> Option<u64> {
        self.observed_axes.get(axis).copied()
    }

    pub(crate) fn required_axes_all_observed(&self) -> bool {
        self.required_axes
            .iter()
            .all(|axis| self.observed_axes.contains_key(axis))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ObservedProviderExecutionEnvelope {
    pub(crate) provider_id: String,
    pub(crate) provider_version: String,
    pub(crate) input_key: String,
    #[serde(skip_serializing)]
    pub(crate) raw_witness_bytes: Vec<u8>,
    pub(crate) raw_witness_sha256: String,
    pub(crate) raw_witness_token: String,
    pub(crate) normalized_record_count: u64,
    pub(crate) actual_budget: ObservedProviderBudget,
    pub(crate) destination_policy_sealed: bool,
    pub(crate) network_hops: Vec<ProviderNetworkHop>,
    pub(crate) observation_state: &'static str,
    pub(crate) coverage_extent: &'static str,
    pub(crate) residual_code: Option<String>,
}

pub(crate) struct ObservedProviderEnvelopeInput<'a> {
    pub(crate) provider_id: String,
    pub(crate) provider_version: String,
    pub(crate) input_key: String,
    pub(crate) raw_witness_bytes: Vec<u8>,
    pub(crate) normalized_record_count: u64,
    pub(crate) request_count: u64,
    pub(crate) wall_clock_ms: u64,
    pub(crate) endpoint: &'a url::Url,
    pub(crate) pinned_addresses: Vec<IpAddr>,
    pub(crate) selected_address: IpAddr,
    pub(crate) exhaustive_empty_contract: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderTransportError {
    DestinationPolicyBlocked,
    #[cfg(test)]
    BudgetExhausted,
    Transport(String),
}

impl ProviderTransportError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::DestinationPolicyBlocked => "TOOL_TRUTH_DESTINATION_POLICY_BLOCKED",
            #[cfg(test)]
            Self::BudgetExhausted => "TOOL_TRUTH_BUDGET_EXHAUSTED",
            Self::Transport(_) => "TOOL_TRUTH_PROVIDER_TRANSPORT_FAILED",
        }
    }
}

impl std::fmt::Display for ProviderTransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DestinationPolicyBlocked => formatter.write_str("provider destination blocked"),
            #[cfg(test)]
            Self::BudgetExhausted => formatter.write_str("provider request budget exhausted"),
            Self::Transport(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ProviderTransportError {}

#[cfg(test)]
pub(crate) trait ToolTruthPinnedTransportV1 {
    fn preflight(&self, endpoint: &url::Url) -> Result<Vec<IpAddr>, ProviderTransportError>;
    fn requested_send_attempts(&self) -> u64 {
        1
    }
    fn send_pinned(
        &self,
        endpoint: &url::Url,
        pinned_addresses: &[IpAddr],
    ) -> Result<Vec<u8>, ProviderTransportError>;
}

pub(crate) fn observed_provider_execution_envelope(
    input: ObservedProviderEnvelopeInput<'_>,
) -> ObservedProviderExecutionEnvelope {
    let ObservedProviderEnvelopeInput {
        provider_id,
        provider_version,
        input_key,
        raw_witness_bytes,
        normalized_record_count,
        request_count,
        wall_clock_ms,
        endpoint,
        pinned_addresses,
        selected_address,
        exhaustive_empty_contract,
    } = input;
    let (observation_state, coverage_extent, residual_code) = if normalized_record_count == 0 {
        if exhaustive_empty_contract {
            ("no_match", "complete", None)
        } else {
            (
                "indeterminate",
                "partial",
                Some("provider_not_exhaustive".to_string()),
            )
        }
    } else {
        ("found", "complete", None)
    };
    let required_axes = ["requests", "response_bytes", "wall_clock_ms", "retries"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let observed_axes = BTreeMap::from([
        ("requests".to_string(), request_count),
        (
            "response_bytes".to_string(),
            u64::try_from(raw_witness_bytes.len()).unwrap_or(u64::MAX),
        ),
        ("wall_clock_ms".to_string(), wall_clock_ms),
        ("retries".to_string(), request_count.saturating_sub(1)),
    ]);
    let raw_witness_sha256 = format!(
        "sha256:{}",
        Sha256::digest(&raw_witness_bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let raw_witness_token = Uuid::new_v4().to_string();
    let registry = OBSERVED_RAW_WITNESSES.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Ok(mut registry) = registry.lock() {
        while registry.len() >= 128 {
            let Some(first) = registry.keys().next().cloned() else {
                break;
            };
            registry.remove(&first);
        }
        registry.insert(raw_witness_token.clone(), raw_witness_bytes.clone());
    }
    ObservedProviderExecutionEnvelope {
        provider_id,
        provider_version,
        input_key,
        raw_witness_bytes,
        raw_witness_sha256,
        raw_witness_token,
        normalized_record_count,
        actual_budget: ObservedProviderBudget {
            required_axes,
            observed_axes,
        },
        destination_policy_sealed: true,
        network_hops: (0..request_count)
            .map(|ordinal| ProviderNetworkHop {
                hop_kind: if ordinal == 0 { "initial" } else { "retry" },
                normalized_host: endpoint.host_str().unwrap_or_default().to_string(),
                url: endpoint.to_string(),
                pinned_addresses: pinned_addresses.clone(),
                selected_address,
                send_ordinal: ordinal.saturating_add(1),
            })
            .collect(),
        observation_state,
        coverage_extent,
        residual_code,
    }
}

static OBSERVED_RAW_WITNESSES: OnceLock<Mutex<BTreeMap<String, Vec<u8>>>> = OnceLock::new();

/// Host-only handoff for receipt sealing. The token is opaque and the bytes are
/// never serialized into the model-visible provider evidence envelope.
pub fn load_observed_raw_witness(token: &str) -> Option<Vec<u8>> {
    OBSERVED_RAW_WITNESSES
        .get()
        .and_then(|registry| registry.lock().ok())
        .and_then(|registry| registry.get(token).cloned())
}

pub fn release_observed_raw_witness(token: &str) {
    if let Some(registry) = OBSERVED_RAW_WITNESSES.get() {
        if let Ok(mut registry) = registry.lock() {
            registry.remove(token);
        }
    }
}

#[cfg(test)]
pub(crate) fn run_provider_with_observed_transport<T: ToolTruthPinnedTransportV1>(
    transport: &T,
    request: ProviderExecutionRequest,
) -> Result<ObservedProviderExecutionEnvelope, ProviderTransportError> {
    let fixed = fixed_provider_endpoint(&request.provider_id)
        .ok_or(ProviderTransportError::DestinationPolicyBlocked)?;
    let mut endpoint = url::Url::parse(&format!(
        "{}://{}:{}{}",
        fixed.scheme, fixed.host, fixed.port, fixed.path
    ))
    .map_err(|error| ProviderTransportError::Transport(error.to_string()))?;
    endpoint
        .query_pairs_mut()
        .append_pair("input", &request.target_input);
    let pinned_addresses = transport.preflight(&endpoint)?;
    if pinned_addresses.is_empty() {
        return Err(ProviderTransportError::DestinationPolicyBlocked);
    }

    let mut sent = 0_u64;
    let mut raw_witness_bytes = Vec::new();
    for _ in 0..transport.requested_send_attempts() {
        if sent >= request.max_requests {
            return Err(ProviderTransportError::BudgetExhausted);
        }
        raw_witness_bytes = transport.send_pinned(&endpoint, &pinned_addresses)?;
        sent += 1;
    }
    let normalized_record_count = serde_json::from_slice::<serde_json::Value>(&raw_witness_bytes)
        .ok()
        .map(|value| match value {
            serde_json::Value::Array(values) => values.len(),
            serde_json::Value::Object(object) => object
                .get("data")
                .or_else(|| object.get("results"))
                .or_else(|| object.get("items"))
                .and_then(serde_json::Value::as_array)
                .map_or_else(|| usize::from(!object.is_empty()), Vec::len),
            _ => 0,
        })
        .unwrap_or(0);
    Ok(observed_provider_execution_envelope(
        ObservedProviderEnvelopeInput {
            provider_id: request.provider_id,
            provider_version: request.provider_version,
            input_key: request.input_key,
            raw_witness_bytes,
            normalized_record_count: u64::try_from(normalized_record_count).unwrap_or(u64::MAX),
            request_count: sent,
            wall_clock_ms: 0,
            endpoint: &endpoint,
            selected_address: pinned_addresses[0],
            pinned_addresses,
            exhaustive_empty_contract: request.exhaustive_empty_contract,
        },
    ))
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderEgressFault {
    MixedPublicAndPrivateDns,
    RedirectOutsideExactAllowlist,
    DnsRebindAfterPin,
}

#[cfg(test)]
pub(crate) struct ScriptedToolTruthPinnedTransport {
    bytes: Vec<u8>,
    fault: Option<ProviderEgressFault>,
    send_attempts: u64,
    sends: std::cell::Cell<u64>,
}

#[cfg(test)]
impl ScriptedToolTruthPinnedTransport {
    pub(crate) fn success(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            fault: None,
            send_attempts: 1,
            sends: std::cell::Cell::new(0),
        }
    }

    pub(crate) fn with_fault(fault: ProviderEgressFault) -> Self {
        Self {
            bytes: Vec::new(),
            fault: Some(fault),
            send_attempts: 1,
            sends: std::cell::Cell::new(0),
        }
    }

    pub(crate) fn with_send_attempts(mut self, send_attempts: u64) -> Self {
        self.send_attempts = send_attempts;
        self
    }

    pub(crate) fn send_count(&self) -> u64 {
        self.sends.get()
    }
}

#[cfg(test)]
impl ToolTruthPinnedTransportV1 for ScriptedToolTruthPinnedTransport {
    fn preflight(&self, _endpoint: &url::Url) -> Result<Vec<IpAddr>, ProviderTransportError> {
        if self.fault.is_some() {
            return Err(ProviderTransportError::DestinationPolicyBlocked);
        }
        Ok(vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))])
    }

    fn requested_send_attempts(&self) -> u64 {
        self.send_attempts
    }

    fn send_pinned(
        &self,
        _endpoint: &url::Url,
        _pinned_addresses: &[IpAddr],
    ) -> Result<Vec<u8>, ProviderTransportError> {
        self.sends.set(self.sends.get().saturating_add(1));
        Ok(self.bytes.clone())
    }
}

#[cfg(test)]
pub(crate) fn provider_request() -> ProviderExecutionRequest {
    ProviderExecutionRequest {
        provider_id: "fixture".to_string(),
        provider_version: "fixture.v1".to_string(),
        input_key: "target-intel-input".to_string(),
        target_input: "example.test".to_string(),
        max_requests: 1,
        exhaustive_empty_contract: true,
    }
}

#[cfg(test)]
mod tool_truth_transport_tests {
    use super::{
        build_pinned_provider_client_with_authority, load_observed_raw_witness,
        prohibited_provider_ip, provider_request, release_observed_raw_witness,
        run_provider_with_observed_transport, validate_provider_destination, ProviderEgressFault,
        ScriptedToolTruthPinnedTransport,
    };
    use golish_pentest::config::ControlledFixtureIntelTransportAuthority;
    use std::net::IpAddr;

    #[test]
    fn provider_transport_returns_host_observed_execution_envelope() {
        let transport = ScriptedToolTruthPinnedTransport::success(b"provider raw bytes");
        let result = run_provider_with_observed_transport(&transport, provider_request())
            .expect("scripted provider request should succeed");
        assert_eq!(result.actual_budget.axis("requests"), Some(1));
        assert!(result.actual_budget.required_axes_all_observed());
        assert_eq!(result.raw_witness_bytes, b"provider raw bytes");
        assert_eq!(
            load_observed_raw_witness(&result.raw_witness_token).as_deref(),
            Some(b"provider raw bytes".as_slice())
        );
        release_observed_raw_witness(&result.raw_witness_token);
        assert!(result.destination_policy_sealed);
        assert_eq!(result.network_hops.len(), 1);
    }

    #[test]
    fn wrapper_empty_response_uses_normalized_count_not_raw_byte_nonemptiness() {
        let transport = ScriptedToolTruthPinnedTransport::success(br#"{"code":0,"data":[]}"#);
        let result = run_provider_with_observed_transport(&transport, provider_request())
            .expect("scripted provider request should succeed");
        assert_eq!(result.normalized_record_count, 0);
        assert_eq!(result.observation_state, "no_match");
    }

    #[test]
    fn ipv4_mapped_private_addresses_are_prohibited() {
        for address in ["::ffff:127.0.0.1", "::ffff:169.254.169.254"] {
            assert!(prohibited_provider_ip(
                address.parse::<IpAddr>().expect("mapped IPv6 fixture")
            ));
        }
    }

    #[tokio::test]
    async fn controlled_fixture_authority_admits_only_its_exact_loopback_endpoint() {
        let authority = ControlledFixtureIntelTransportAuthority::loopback_http(
            url::Url::parse("http://127.0.0.1:32123/intel/company.json").unwrap(),
        )
        .unwrap();
        let endpoint = validate_provider_destination(
            authority.provider_id(),
            "http://127.0.0.1:32123/intel/company.json?company={{company_name}}",
            "http://127.0.0.1:32123/intel/company.json?company=Golish",
            Some(&authority),
        )
        .expect("exact controlled endpoint is admitted");
        let (_, addresses, selected) =
            build_pinned_provider_client_with_authority(&endpoint, None, Some(&authority))
                .await
                .expect("exact literal loopback is pinned");
        assert_eq!(addresses, vec!["127.0.0.1".parse::<IpAddr>().unwrap()]);
        assert_eq!(selected, "127.0.0.1".parse::<IpAddr>().unwrap());

        for rendered in [
            "http://127.0.0.1:32124/intel/company.json?company=Golish",
            "http://127.0.0.1:32123/intel/other.json?company=Golish",
            "http://127.0.0.1:32123@attacker.example/intel/company.json",
        ] {
            assert!(validate_provider_destination(
                authority.provider_id(),
                "http://127.0.0.1:32123/intel/company.json?company={{company_name}}",
                rendered,
                Some(&authority),
            )
            .is_err());
        }
        assert!(validate_provider_destination(
            authority.provider_id(),
            "http://127.0.0.1:32123/intel/company.json?company={{company_name}}",
            "http://127.0.0.1:32123/intel/company.json?company=Golish",
            None,
        )
        .is_err());
    }

    #[test]
    fn provider_transport_cannot_turn_target_input_into_destination_authority() {
        let transport = ScriptedToolTruthPinnedTransport::success(b"{}");
        let mut request = provider_request();
        request.target_input = "https://169.254.169.254/latest/meta-data".to_string();
        let envelope = run_provider_with_observed_transport(&transport, request)
            .expect("target input is escaped into the fixed provider request");
        assert_eq!(
            envelope.network_hops[0].normalized_host,
            "fixed.provider.example.test"
        );
        assert!(!envelope.network_hops[0]
            .url
            .contains("169.254.169.254/latest/meta-data"));
    }

    #[test]
    fn provider_transport_blocks_mixed_dns_redirect_and_rebinding_before_send() {
        for fault in [
            ProviderEgressFault::MixedPublicAndPrivateDns,
            ProviderEgressFault::RedirectOutsideExactAllowlist,
            ProviderEgressFault::DnsRebindAfterPin,
        ] {
            let transport = ScriptedToolTruthPinnedTransport::with_fault(fault);
            let error = run_provider_with_observed_transport(&transport, provider_request())
                .expect_err("every hop is re-authorized and pinned");
            assert_eq!(error.code(), "TOOL_TRUTH_DESTINATION_POLICY_BLOCKED");
            assert_eq!(transport.send_count(), 0);
        }
    }

    #[test]
    fn provider_request_n_plus_one_is_rejected_before_send() {
        let transport = ScriptedToolTruthPinnedTransport::success(b"{}").with_send_attempts(2);
        let error = run_provider_with_observed_transport(&transport, provider_request())
            .expect_err("request N+1 must be rejected before transport send");
        assert_eq!(error.code(), "TOOL_TRUTH_BUDGET_EXHAUSTED");
        assert_eq!(transport.send_count(), 1);
    }

    #[test]
    fn provider_empty_requires_exhaustive_contract() {
        let transport = ScriptedToolTruthPinnedTransport::success(b"[]");
        let mut request = provider_request();
        request.exhaustive_empty_contract = false;
        let envelope = run_provider_with_observed_transport(&transport, request)
            .expect("empty provider response remains auditable");
        assert_eq!(envelope.observation_state, "indeterminate");
        assert_eq!(envelope.coverage_extent, "partial");
        assert_eq!(
            envelope.residual_code.as_deref(),
            Some("provider_not_exhaustive")
        );
    }
}

fn parse_query_type(s: &str) -> Result<QueryType, GolishError> {
    match s {
        "site" => Ok(QueryType::Site),
        "domain" => Ok(QueryType::Domain),
        "email" => Ok(QueryType::Email),
        "apk" => Ok(QueryType::Apk),
        "sensitive" => Ok(QueryType::Sensitive),
        "code" => Ok(QueryType::Code),
        "member" => Ok(QueryType::Member),
        "org" => Ok(QueryType::Org),
        "branch" => Ok(QueryType::Branch),
        "darknet" => Ok(QueryType::Darknet),
        "cert" => Ok(QueryType::Cert),
        "asn" => Ok(QueryType::Asn),
        "cidr" => Ok(QueryType::Cidr),
        other => Err(GolishError::Validation(format!(
            "unknown query_type: {other}"
        ))),
    }
}

/// List all registered ASM intel providers and their static metadata.
///
/// Settings UI calls this on mount to render one card per provider.
#[tauri::command]
pub async fn intel_list_providers() -> Result<Vec<ProviderMeta>, GolishError> {
    let reg = provider_registry();
    let mut metas: Vec<ProviderMeta> = reg.values().map(|p| p.meta()).collect();
    // Stable ordering by id so the UI doesn't reshuffle on every call.
    metas.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(metas)
}

/// Test whether the configured API key for `provider_id` is valid.
///
/// Settings UI calls this from the "Test Connection" button on each
/// provider card.
#[tauri::command]
pub async fn intel_test_connection(
    state: tauri::State<'_, DbState>,
    provider_id: String,
) -> Result<ConnectionStatus, GolishError> {
    let pool = state.pool_ready().await?;
    let reg = provider_registry();
    let provider = reg
        .get(&provider_id)
        .ok_or_else(|| GolishError::NotFound(format!("intel provider '{provider_id}'")))?;

    let store = PgVaultKeyStore { pool: pool.clone() };
    let key = store
        .get_key(&provider_id)
        .await
        .map_err(|e| GolishError::Internal(e.to_string()))?
        .unwrap_or_default();

    provider
        .test_connection(&key)
        .await
        .map_err(|e| GolishError::Internal(e.to_string()))
}

/// Run an ASM intel query and persist results into `organizations`.
///
/// Returns the parsed `ProviderRecord`s so the UI can also display them.
/// Results are written into `organizations` via
/// `output_store::store_organization_update` before this returns.
#[tauri::command]
pub async fn intel_query_provider(
    state: tauri::State<'_, DbState>,
    provider_id: String,
    query_type: String,
    query: String,
    project_path: Option<String>,
) -> Result<IntelQueryResult, GolishError> {
    let pool = state.pool_ready().await?;
    let reg = provider_registry();
    let provider = reg
        .get(&provider_id)
        .ok_or_else(|| GolishError::NotFound(format!("intel provider '{provider_id}'")))?;

    let qt = parse_query_type(&query_type)?;
    let store = PgVaultKeyStore { pool: pool.clone() };
    let key = store
        .get_key(&provider_id)
        .await
        .map_err(|e| GolishError::Internal(e.to_string()))?
        .ok_or_else(|| {
            GolishError::Config(format!(
                "no API key configured for provider '{provider_id}' (Settings → Intel Providers)"
            ))
        })?;

    let records: Vec<ProviderRecord> = provider
        .query(qt, &query, &key)
        .await
        .map_err(|e| GolishError::Internal(e.to_string()))?;

    let pg_store = golish_pentest::output_store::PgPentestStore::new(pool);
    let mut persisted: usize = 0;
    let mut errors: Vec<String> = Vec::new();
    let mut targets_written: usize = 0;
    for record in &records {
        // Enrich with provider + query_type meta keys so the writer can
        // bucket leftover fields into organizations.intel.records[]
        // (see organizations.rs docstring for the meta-key convention).
        let mut enriched: HashMap<String, String> = record.fields.clone();
        enriched.insert("_provider".into(), record.provider.clone());
        enriched.insert("_query_type".into(), record.query_type.as_str().into());

        // Step 1 (always): write into organizations.* + intel.records[] catch-all.
        match pg_store
            .store_organization_update(&enriched, project_path.as_deref())
            .await
        {
            Ok(()) => persisted += 1,
            Err(e) => errors.push(format!("organization_update: {e}")),
        }

        // Step 2 (conditional): when the record carries asset-level fields
        // (ip / port / title / webserver / ...), also persist them into the
        // `targets` table so the Asset / Recon UI surfaces them instead of
        // leaving everything buried under organizations.intel.records[].
        if let Some(target_fields) = build_target_fields_from_intel(&enriched) {
            let host_val = target_fields
                .get("host")
                .cloned()
                .or_else(|| target_fields.get("ip").cloned())
                .or_else(|| target_fields.get("url").cloned())
                .unwrap_or_default();
            if host_val.trim().is_empty() {
                continue;
            }
            let target_id = match pg_store
                .find_or_create_target(&host_val, project_path.as_deref())
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    errors.push(format!("target_find: {e}"));
                    continue;
                }
            };
            let tool_name = format!("intel/{}/{}", record.provider, record.query_type.as_str());
            if let Err(e) = pg_store
                .store_target_update_recon(&target_fields, project_path.as_deref(), &tool_name)
                .await
            {
                errors.push(format!("target_update_recon: {e}"));
            } else {
                targets_written += 1;
            }
            if let Some(org_name) = enriched.get("organization_name").map(|s| s.trim()) {
                if !org_name.is_empty() {
                    if let Err(e) = link_target_to_organization(
                        pool,
                        target_id,
                        org_name,
                        project_path.as_deref(),
                    )
                    .await
                    {
                        errors.push(format!("target_link_org: {e}"));
                    }
                }
            }
        }
    }
    tracing::info!(
        "[intel_query_provider] provider={} qt={} records={} orgs_persisted={} targets_written={} errors={}",
        provider_id,
        query_type,
        records.len(),
        persisted,
        targets_written,
        errors.len(),
    );

    Ok(IntelQueryResult {
        provider: provider_id,
        query_type,
        records,
        persisted,
        targets_written,
        errors,
    })
}

/// Derive a `fields` map suitable for `store_target_update_recon` from an
/// intel-provider record. Returns `None` when the record lacks any
/// host-identifying key (e.g. a 0.zone `member` or `email` record that
/// has no asset surface).
///
/// Side effects on the returned map:
/// - If `host` is absent but `domain` is present, copy `domain → host` so
///   the writer picks the most stable identifier (avoids spawning a `1.2.3.4`
///   IP target alongside a `api.example.com` domain target for the same asset).
fn build_target_fields_from_intel(
    fields: &HashMap<String, String>,
) -> Option<HashMap<String, String>> {
    let has_asset = ["host", "ip", "url", "domain"].iter().any(|k| {
        fields
            .get(*k)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    });
    if !has_asset {
        return None;
    }
    let mut out = fields.clone();
    let host_blank = out.get("host").map(|s| s.trim().is_empty()).unwrap_or(true);
    if host_blank {
        if let Some(domain) = out.get("domain").cloned() {
            if !domain.trim().is_empty() {
                out.insert("host".into(), domain);
            }
        }
    }
    Some(out)
}

/// Idempotently attach a freshly-created/updated target to the root
/// organization named `organization_name` (matching the find-or-create
/// rule used by `store_organization_update`).
///
/// Best-effort: returns Ok even when the organization row hasn't been
/// created yet — in that case `targets.organization_id` stays NULL and
/// will be populated on the next intel write.
async fn link_target_to_organization(
    pool: &PgPool,
    target_id: Uuid,
    organization_name: &str,
    project_path: Option<&str>,
) -> anyhow::Result<()> {
    let pp = project_path.unwrap_or("");
    let org_id =
        golish_db::repo::organizations::find_root_id_by_name(pool, pp, organization_name).await?;
    let Some(oid) = org_id else { return Ok(()) };

    sqlx::query(
        r#"UPDATE targets
           SET organization_id = $1
           WHERE id = $2 AND organization_id IS NULL"#,
    )
    .bind(oid)
    .bind(target_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct IntelQueryResult {
    pub provider: String,
    pub query_type: String,
    pub records: Vec<ProviderRecord>,
    /// How many records were successfully persisted to `organizations`.
    pub persisted: usize,
    /// How many records also produced an asset row in the `targets` table
    /// (records lacking any host/ip/url/domain key are skipped here).
    pub targets_written: usize,
    /// Per-record persistence errors (non-fatal — surfaced so UI can warn).
    pub errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn build_target_fields_returns_none_when_no_asset_key() {
        // 0.zone "member" / "email" / "code" records carry no host/ip/url/domain
        let fields = h(&[
            ("organization_name", "Acme"),
            ("contact_name", "Alice"),
            ("contact_source", "linkedin"),
        ]);
        assert!(build_target_fields_from_intel(&fields).is_none());
    }

    #[test]
    fn build_target_fields_copies_domain_to_host_when_host_missing() {
        // Shodan / Quake commonly emit `ip + domain` without an explicit `host`.
        // We want the resulting target keyed on the most stable identifier (the
        // domain), so the writer doesn't spawn separate IP-typed and
        // domain-typed targets for the same asset.
        let fields = h(&[
            ("ip", "1.2.3.4"),
            ("domain", "api.example.com"),
            ("port", "443"),
            ("title", "Hello"),
        ]);
        let out = build_target_fields_from_intel(&fields).expect("has ip");
        assert_eq!(out.get("host").map(String::as_str), Some("api.example.com"));
        assert_eq!(out.get("ip").map(String::as_str), Some("1.2.3.4"));
        assert_eq!(out.get("port").map(String::as_str), Some("443"));
    }

    #[test]
    fn build_target_fields_keeps_explicit_host() {
        // FOFA emits `host` as a full URL; let the downstream writer normalize.
        let fields = h(&[
            ("host", "https://example.com"),
            ("ip", "93.184.216.34"),
            ("domain", "example.com"),
        ]);
        let out = build_target_fields_from_intel(&fields).expect("has host");
        assert_eq!(
            out.get("host").map(String::as_str),
            Some("https://example.com")
        );
    }

    #[test]
    fn build_target_fields_accepts_domain_only() {
        // 0.zone `domain` query → only the `domain` key is set.
        let fields = h(&[("domain", "sub.example.com"), ("organization_name", "Acme")]);
        let out = build_target_fields_from_intel(&fields).expect("has domain");
        assert_eq!(out.get("host").map(String::as_str), Some("sub.example.com"));
        assert_eq!(
            out.get("domain").map(String::as_str),
            Some("sub.example.com")
        );
    }

    #[test]
    fn build_target_fields_treats_blank_values_as_missing() {
        let fields = h(&[("ip", "   "), ("host", "")]);
        assert!(build_target_fields_from_intel(&fields).is_none());
    }

    #[test]
    fn build_target_fields_does_not_overwrite_existing_host_with_domain() {
        // host is set (even if it's a URL); leave it alone.
        let fields = h(&[("host", "https://a.com"), ("domain", "b.com")]);
        let out = build_target_fields_from_intel(&fields).expect("has host");
        assert_eq!(out.get("host").map(String::as_str), Some("https://a.com"));
    }
}
