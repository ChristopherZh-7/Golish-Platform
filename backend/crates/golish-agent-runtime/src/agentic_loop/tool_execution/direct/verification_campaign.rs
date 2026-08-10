//! Per-hop authority and budget gate for Plan C trusted transports.
//!
//! The actual adapter may send only after receiving `AuthorizedPinnedSend`.
//! This module performs no I/O; callers must re-read the snapshots passed here
//! immediately before every initial connection, redirect and retry.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetAxisV1 {
    Requests,
    ResponseBytes,
    WallClockMs,
    Retries,
    BrowserSteps,
    OastTokens,
}

impl BudgetAxisV1 {
    pub const ALL: [Self; 6] = [
        Self::Requests,
        Self::ResponseBytes,
        Self::WallClockMs,
        Self::Retries,
        Self::BrowserSteps,
        Self::OastTokens,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetHeadSnapshotV1 {
    pub limits: BTreeMap<BudgetAxisV1, u64>,
    pub consumed: BTreeMap<BudgetAxisV1, u64>,
    pub reserved: BTreeMap<BudgetAxisV1, u64>,
    pub unknown_held: BTreeMap<BudgetAxisV1, u64>,
    /// Remaining upper bound belonging to this exact action reservation.
    pub reservation_remaining: BTreeMap<BudgetAxisV1, u64>,
    pub fences: BTreeMap<BudgetAxisV1, i64>,
}

impl BudgetHeadSnapshotV1 {
    fn complete(&self) -> bool {
        BudgetAxisV1::ALL.iter().all(|axis| {
            self.limits.get(axis).is_some_and(|limit| *limit > 0)
                && self.consumed.contains_key(axis)
                && self.reserved.contains_key(axis)
                && self.unknown_held.contains_key(axis)
                && self.reservation_remaining.contains_key(axis)
                && self.fences.get(axis).is_some_and(|fence| *fence >= 0)
        })
    }

    fn permits(&self, delta: &BTreeMap<BudgetAxisV1, u64>) -> bool {
        self.complete()
            && delta
                .get(&BudgetAxisV1::Requests)
                .is_some_and(|requests| *requests > 0)
            && delta.keys().all(|axis| BudgetAxisV1::ALL.contains(axis))
            && BudgetAxisV1::ALL.iter().all(|axis| {
                let limit = self.limits[axis];
                let consumed = self.consumed[axis];
                let reserved = self.reserved[axis];
                let unknown = self.unknown_held[axis];
                let increment = delta.get(axis).copied().unwrap_or(0);
                consumed
                    .checked_add(reserved)
                    .and_then(|value| value.checked_add(unknown))
                    .is_some_and(|current| current <= limit)
                    && self.reservation_remaining[axis] <= reserved
                    && increment <= self.reservation_remaining[axis]
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchicalBudgetSnapshotV1 {
    pub operation: BudgetHeadSnapshotV1,
    pub wave: BudgetHeadSnapshotV1,
    pub campaign: BudgetHeadSnapshotV1,
    pub action: BudgetHeadSnapshotV1,
}

impl HierarchicalBudgetSnapshotV1 {
    fn permits(&self, delta: &BTreeMap<BudgetAxisV1, u64>) -> bool {
        [&self.operation, &self.wave, &self.campaign, &self.action]
            .into_iter()
            .all(|head| head.permits(delta))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignDispatchAuthoritySnapshotV1 {
    pub campaign_dispatch_held: bool,
    pub campaign_dispatch_generation: i64,
    pub operation_admission_held: bool,
    pub operation_admission_generation: i64,
    pub global_row_version: i64,
    pub quarantine_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAuthoritySnapshotV1 {
    pub handle_version: u32,
    pub revocation_generation: i64,
    pub revoked: bool,
    pub injection_origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenSendAuthorizationV1 {
    pub campaign_dispatch_generation: i64,
    pub credential_handle_version: Option<u32>,
    pub credential_revocation_generation: Option<i64>,
    pub exact_origin: String,
    pub path_boundary: String,
    pub allowed_destination_origins: BTreeSet<String>,
    pub max_redirect_hops: u8,
    pub allow_non_public_destination: bool,
    pub non_public_scope_exception_hash: Option<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextSendRequestV1 {
    pub requested_url: String,
    pub redirect_hop: u8,
    pub is_retry: bool,
    pub resolved_addresses: Vec<IpAddr>,
    pub selected_address: IpAddr,
    pub budget_delta: BTreeMap<BudgetAxisV1, u64>,
    pub now: DateTime<Utc>,
}

/// Durable identities from a committed begin receipt. Supplying identifiers is
/// only a selector: the configured host repository re-derives every authority
/// field and rejects mismatched ownership/CAS relations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedActionSendSelectorV1 {
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub prepared_action_id: Uuid,
    pub authorization_receipt_id: Uuid,
    pub action_execution_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPerHopAuthorityContextV1 {
    pub authorization: FrozenSendAuthorizationV1,
    pub dispatch: CampaignDispatchAuthoritySnapshotV1,
    pub credential: Option<CredentialAuthoritySnapshotV1>,
    pub budgets: HierarchicalBudgetSnapshotV1,
    /// Strong DB/current-host time; callers cannot choose authorization time.
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostNextSendRequestV1 {
    pub requested_url: String,
    pub redirect_hop: u8,
    pub is_retry: bool,
    pub budget_delta: BTreeMap<BudgetAxisV1, u64>,
}

#[async_trait::async_trait]
pub trait HostPerHopAuthorityRepository: Send + Sync {
    /// Strongly re-read campaign hold/generation, quarantine, credential,
    /// authorization expiry/policy and all four budget heads.
    async fn read_current_send_authority(
        &self,
        selector: PreparedActionSendSelectorV1,
    ) -> Result<HostPerHopAuthorityContextV1, SendAuthorityError>;

    /// Atomically consumes the already-reserved delta on the fixed ancestor
    /// order operation -> wave -> campaign -> action. Implementations must
    /// recheck every fence and the campaign dispatch generation in the same
    /// short transaction, before returning the capability token to the host.
    async fn consume_budget_before_send(
        &self,
        selector: PreparedActionSendSelectorV1,
        expected_campaign_dispatch_generation: i64,
        expected_budget_fences: [BTreeMap<BudgetAxisV1, i64>; 4],
        delta: &BTreeMap<BudgetAxisV1, u64>,
    ) -> Result<(), SendAuthorityError>;
}

#[async_trait::async_trait]
pub trait HostPinnedResolver: Send + Sync {
    /// Returns the complete A/AAAA answer set for this connection attempt.
    async fn resolve_all(
        &self,
        canonical_host: &str,
        canonical_port: u16,
    ) -> Result<Vec<IpAddr>, SendAuthorityError>;
}

/// System resolver used by the production host. The complete answer set is
/// returned to the policy gate; it never selects or suppresses an address.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemPinnedResolver;

#[async_trait::async_trait]
impl HostPinnedResolver for SystemPinnedResolver {
    async fn resolve_all(
        &self,
        canonical_host: &str,
        canonical_port: u16,
    ) -> Result<Vec<IpAddr>, SendAuthorityError> {
        let addresses = tokio::net::lookup_host((canonical_host, canonical_port))
            .await
            .map_err(|_| SendAuthorityError::DnsPolicyDenied)?;
        Ok(addresses.map(|address| address.ip()).collect())
    }
}

/// Host-owned per-hop boundary. It accepts no cached authority token, caller
/// clock, caller-selected address or caller-provided DNS answer. The output is
/// created only after a strong authority read, complete DNS validation and an
/// atomic pre-send budget consumption.
pub async fn authorize_next_pinned_send_from_host(
    repository: &dyn HostPerHopAuthorityRepository,
    resolver: &dyn HostPinnedResolver,
    selector: PreparedActionSendSelectorV1,
    request: HostNextSendRequestV1,
) -> Result<AuthorizedPinnedSend, SendAuthorityError> {
    if [
        selector.operation_id,
        selector.campaign_id,
        selector.prepared_action_id,
        selector.authorization_receipt_id,
        selector.action_execution_id,
    ]
    .into_iter()
    .any(|id| id.is_nil())
    {
        return Err(SendAuthorityError::AuthorityQuarantined);
    }
    let context = repository.read_current_send_authority(selector).await?;
    let url = Url::parse(&request.requested_url)
        .map_err(|_| SendAuthorityError::DestinationPolicyDenied)?;
    let host = url
        .host_str()
        .ok_or(SendAuthorityError::DestinationPolicyDenied)?
        .to_ascii_lowercase();
    let port = url
        .port_or_known_default()
        .ok_or(SendAuthorityError::DestinationPolicyDenied)?;
    let mut resolved_addresses = resolver.resolve_all(&host, port).await?;
    resolved_addresses.sort();
    resolved_addresses.dedup();
    let selected_address = *resolved_addresses
        .first()
        .ok_or(SendAuthorityError::DnsPolicyDenied)?;
    let pure_request = NextSendRequestV1 {
        requested_url: request.requested_url,
        redirect_hop: request.redirect_hop,
        is_retry: request.is_retry,
        resolved_addresses,
        selected_address,
        budget_delta: request.budget_delta,
        now: context.checked_at,
    };
    let token = authorize_next_pinned_send(
        &context.authorization,
        &context.dispatch,
        context.credential.as_ref(),
        &context.budgets,
        &pure_request,
    )?;
    repository
        .consume_budget_before_send(
            selector,
            context.authorization.campaign_dispatch_generation,
            [
                context.budgets.operation.fences.clone(),
                context.budgets.wave.fences.clone(),
                context.budgets.campaign.fences.clone(),
                context.budgets.action.fences.clone(),
            ],
            &pure_request.budget_delta,
        )
        .await?;
    Ok(token)
}

/// Capability token handed to the pinned transport. Its fields are private and
/// no public constructor exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedPinnedSend {
    requested_url: String,
    pinned_address: IpAddr,
    canonical_host: String,
    canonical_port: u16,
    redirect_hop: u8,
    strip_authorization_and_cookies: bool,
    credential_required: bool,
}

impl AuthorizedPinnedSend {
    pub fn requested_url(&self) -> &str {
        &self.requested_url
    }
    pub const fn pinned_address(&self) -> IpAddr {
        self.pinned_address
    }
    pub fn canonical_host(&self) -> &str {
        &self.canonical_host
    }
    pub const fn canonical_port(&self) -> u16 {
        self.canonical_port
    }
    pub const fn redirect_hop(&self) -> u8 {
        self.redirect_hop
    }
    pub const fn strip_authorization_and_cookies(&self) -> bool {
        self.strip_authorization_and_cookies
    }
    pub const fn credential_required(&self) -> bool {
        self.credential_required
    }
}

/// Host-only credential decorator. Implementations resolve the opaque vault
/// handle from the durable action selector and may only decorate the already
/// policy-authorized request for the exact canonical origin. The runtime never
/// accepts a model-provided header or secret value.
#[async_trait::async_trait]
pub trait HostCredentialInjector: Send + Sync {
    async fn inject_exact_origin(
        &self,
        selector: PreparedActionSendSelectorV1,
        exact_origin: &str,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, SendAuthorityError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustedGetLimitsV1 {
    pub max_response_bytes_per_hop: u64,
    pub max_wall_clock_ms_per_hop: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedHttpHopObservationV1 {
    pub url: String,
    pub status: u16,
    pub response_bytes: u64,
    pub body_sha256: String,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedHttpObservationV1 {
    pub final_url: String,
    pub hops: Vec<TrustedHttpHopObservationV1>,
}

/// Executes one bounded GET chain. Every hop receives a fresh authority read,
/// full DNS validation, pinned socket address and atomic reservation consume.
/// Redirects are disabled in reqwest and followed only by this loop after the
/// next hop passes the same host gate. No ambient proxy or cookie jar exists.
pub async fn execute_trusted_get_v1(
    repository: &dyn HostPerHopAuthorityRepository,
    resolver: &dyn HostPinnedResolver,
    credential_injector: Option<&dyn HostCredentialInjector>,
    selector: PreparedActionSendSelectorV1,
    initial_url: String,
    limits: TrustedGetLimitsV1,
) -> Result<TrustedHttpObservationV1, SendAuthorityError> {
    if limits.max_response_bytes_per_hop == 0 || limits.max_wall_clock_ms_per_hop == 0 {
        return Err(SendAuthorityError::BudgetExhausted);
    }
    let mut next_url = initial_url;
    let mut redirect_hop = 0_u8;
    let mut observations = Vec::new();
    loop {
        let budget_delta = [
            (BudgetAxisV1::Requests, 1),
            (
                BudgetAxisV1::ResponseBytes,
                limits.max_response_bytes_per_hop,
            ),
            (BudgetAxisV1::WallClockMs, limits.max_wall_clock_ms_per_hop),
            (BudgetAxisV1::Retries, u64::from(redirect_hop > 0)),
        ]
        .into_iter()
        .collect();
        let authorized = authorize_next_pinned_send_from_host(
            repository,
            resolver,
            selector,
            HostNextSendRequestV1 {
                requested_url: next_url.clone(),
                redirect_hop,
                is_retry: redirect_hop > 0,
                budget_delta,
            },
        )
        .await?;
        let pinned =
            std::net::SocketAddr::new(authorized.pinned_address(), authorized.canonical_port());
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_millis(limits.max_wall_clock_ms_per_hop))
            .timeout(Duration::from_millis(limits.max_wall_clock_ms_per_hop))
            .resolve(authorized.canonical_host(), pinned)
            .build()
            .map_err(|_| SendAuthorityError::DestinationPolicyDenied)?;
        let mut request = client.get(authorized.requested_url()).header(
            reqwest::header::USER_AGENT,
            "Golish-Verification-Campaign/1",
        );
        if authorized.credential_required() {
            let injector = credential_injector.ok_or(SendAuthorityError::CredentialDrift)?;
            let exact_origin = Url::parse(authorized.requested_url())
                .map_err(|_| SendAuthorityError::DestinationPolicyDenied)?
                .origin()
                .ascii_serialization();
            request = injector
                .inject_exact_origin(selector, &exact_origin, request)
                .await?;
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| SendAuthorityError::DestinationPolicyDenied)?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut byte_count = 0_u64;
        let mut body_hash = Sha256::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| SendAuthorityError::DestinationPolicyDenied)?
        {
            byte_count = byte_count
                .checked_add(
                    u64::try_from(chunk.len()).map_err(|_| SendAuthorityError::BudgetExhausted)?,
                )
                .ok_or(SendAuthorityError::BudgetExhausted)?;
            if byte_count > limits.max_response_bytes_per_hop {
                return Err(SendAuthorityError::BudgetExhausted);
            }
            body_hash.update(&chunk);
        }
        observations.push(TrustedHttpHopObservationV1 {
            url: authorized.requested_url().to_owned(),
            status: status.as_u16(),
            response_bytes: byte_count,
            body_sha256: format!(
                "sha256:{}",
                body_hash
                    .finalize()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
            content_type,
        });
        if !status.is_redirection() {
            return Ok(TrustedHttpObservationV1 {
                final_url: authorized.requested_url().to_owned(),
                hops: observations,
            });
        }
        let location = location.ok_or(SendAuthorityError::DestinationPolicyDenied)?;
        next_url = Url::parse(authorized.requested_url())
            .and_then(|base| base.join(&location))
            .map_err(|_| SendAuthorityError::DestinationPolicyDenied)?
            .to_string();
        redirect_hop = redirect_hop
            .checked_add(1)
            .ok_or(SendAuthorityError::DestinationPolicyDenied)?;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SendAuthorityError {
    #[error("VERIFICATION_CAMPAIGN_DISPATCH_HELD")]
    CampaignDispatchHeld,
    #[error("VERIFICATION_CAMPAIGN_DISPATCH_GENERATION_DRIFT")]
    CampaignDispatchGenerationDrift,
    #[error("VERIFICATION_CAMPAIGN_AUTHORITY_QUARANTINED")]
    AuthorityQuarantined,
    #[error("VERIFICATION_CAMPAIGN_AUTHORIZATION_EXPIRED")]
    AuthorizationExpired,
    #[error("VERIFICATION_CAMPAIGN_CREDENTIAL_DRIFT")]
    CredentialDrift,
    #[error("VERIFICATION_CAMPAIGN_DESTINATION_POLICY_DENIED")]
    DestinationPolicyDenied,
    #[error("VERIFICATION_CAMPAIGN_DNS_POLICY_DENIED")]
    DnsPolicyDenied,
    #[error("VERIFICATION_CAMPAIGN_BUDGET_EXHAUSTED")]
    BudgetExhausted,
}

impl SendAuthorityError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::CampaignDispatchHeld => "VERIFICATION_CAMPAIGN_DISPATCH_HELD",
            Self::CampaignDispatchGenerationDrift => {
                "VERIFICATION_CAMPAIGN_DISPATCH_GENERATION_DRIFT"
            }
            Self::AuthorityQuarantined => "VERIFICATION_CAMPAIGN_AUTHORITY_QUARANTINED",
            Self::AuthorizationExpired => "VERIFICATION_CAMPAIGN_AUTHORIZATION_EXPIRED",
            Self::CredentialDrift => "VERIFICATION_CAMPAIGN_CREDENTIAL_DRIFT",
            Self::DestinationPolicyDenied => "VERIFICATION_CAMPAIGN_DESTINATION_POLICY_DENIED",
            Self::DnsPolicyDenied => "VERIFICATION_CAMPAIGN_DNS_POLICY_DENIED",
            Self::BudgetExhausted => "VERIFICATION_CAMPAIGN_BUDGET_EXHAUSTED",
        }
    }
}

pub fn authorize_next_pinned_send(
    authorization: &FrozenSendAuthorizationV1,
    dispatch: &CampaignDispatchAuthoritySnapshotV1,
    credential: Option<&CredentialAuthoritySnapshotV1>,
    budgets: &HierarchicalBudgetSnapshotV1,
    request: &NextSendRequestV1,
) -> Result<AuthorizedPinnedSend, SendAuthorityError> {
    // Exact order: authority/credential checks precede URL/DNS, secret
    // injection, budget consumption and network I/O.
    if dispatch.campaign_dispatch_held {
        return Err(SendAuthorityError::CampaignDispatchHeld);
    }
    if dispatch.operation_admission_held {
        return Err(SendAuthorityError::CampaignDispatchHeld);
    }
    if dispatch.campaign_dispatch_generation != authorization.campaign_dispatch_generation {
        return Err(SendAuthorityError::CampaignDispatchGenerationDrift);
    }
    if dispatch.quarantine_pending {
        return Err(SendAuthorityError::AuthorityQuarantined);
    }
    if request.now >= authorization.expires_at {
        return Err(SendAuthorityError::AuthorizationExpired);
    }
    validate_credential(authorization, credential)?;

    let url = Url::parse(&request.requested_url)
        .map_err(|_| SendAuthorityError::DestinationPolicyDenied)?;
    let host = url
        .host_str()
        .ok_or(SendAuthorityError::DestinationPolicyDenied)?;
    let port = url
        .port_or_known_default()
        .ok_or(SendAuthorityError::DestinationPolicyDenied)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.username() != ""
        || url.password().is_some()
        || url.fragment().is_some()
        || request.redirect_hop > authorization.max_redirect_hops
        || !authorization
            .allowed_destination_origins
            .contains(&url.origin().ascii_serialization())
        || !path_within_boundary(url.path(), &authorization.path_boundary)
    {
        return Err(SendAuthorityError::DestinationPolicyDenied);
    }

    if request.resolved_addresses.is_empty()
        || !request
            .resolved_addresses
            .contains(&request.selected_address)
        || request
            .resolved_addresses
            .iter()
            .any(|address| address_forbidden(*address))
            && !valid_non_public_exception(authorization)
    {
        return Err(SendAuthorityError::DnsPolicyDenied);
    }
    if !budgets.permits(&request.budget_delta) {
        return Err(SendAuthorityError::BudgetExhausted);
    }

    let origin_changed = url.origin().ascii_serialization() != authorization.exact_origin;
    Ok(AuthorizedPinnedSend {
        requested_url: request.requested_url.clone(),
        pinned_address: request.selected_address,
        canonical_host: host.to_ascii_lowercase(),
        canonical_port: port,
        redirect_hop: request.redirect_hop,
        strip_authorization_and_cookies: origin_changed || request.is_retry,
        credential_required: authorization.credential_handle_version.is_some(),
    })
}

fn validate_credential(
    authorization: &FrozenSendAuthorizationV1,
    credential: Option<&CredentialAuthoritySnapshotV1>,
) -> Result<(), SendAuthorityError> {
    match (
        authorization.credential_handle_version,
        authorization.credential_revocation_generation,
        credential,
    ) {
        (None, None, None) => Ok(()),
        (Some(version), Some(generation), Some(current))
            if !current.revoked
                && current.handle_version == version
                && current.revocation_generation == generation
                && current.injection_origin == authorization.exact_origin =>
        {
            Ok(())
        }
        _ => Err(SendAuthorityError::CredentialDrift),
    }
}

fn path_within_boundary(path: &str, boundary: &str) -> bool {
    if boundary == "/" {
        return true;
    }
    let normalized = boundary.trim_end_matches('/');
    path == normalized
        || path
            .strip_prefix(normalized)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn valid_non_public_exception(authorization: &FrozenSendAuthorizationV1) -> bool {
    authorization.allow_non_public_destination
        && authorization
            .non_public_scope_exception_hash
            .as_deref()
            .is_some_and(valid_hash)
}

fn valid_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn address_forbidden(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            address.is_loopback()
                || address.is_private()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_broadcast()
                || address.octets() == [169, 254, 169, 254]
        }
        IpAddr::V6(address) => {
            address.is_loopback()
                || address.is_unspecified()
                || (address.segments()[0] & 0xfe00) == 0xfc00
                || (address.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn hash(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn head(limit: u64) -> BudgetHeadSnapshotV1 {
        let limits = BudgetAxisV1::ALL
            .into_iter()
            .map(|axis| (axis, limit))
            .collect();
        let zero: BTreeMap<BudgetAxisV1, u64> = BudgetAxisV1::ALL
            .into_iter()
            .map(|axis| (axis, 0))
            .collect();
        BudgetHeadSnapshotV1 {
            limits,
            consumed: zero.clone(),
            reserved: BudgetAxisV1::ALL
                .into_iter()
                .map(|axis| (axis, limit))
                .collect(),
            unknown_held: zero,
            reservation_remaining: BudgetAxisV1::ALL
                .into_iter()
                .map(|axis| (axis, limit))
                .collect(),
            fences: BudgetAxisV1::ALL
                .into_iter()
                .map(|axis| (axis, 1))
                .collect(),
        }
    }

    fn fixture() -> (
        FrozenSendAuthorizationV1,
        CampaignDispatchAuthoritySnapshotV1,
        HierarchicalBudgetSnapshotV1,
        NextSendRequestV1,
    ) {
        let now = Utc::now();
        let authorization = FrozenSendAuthorizationV1 {
            campaign_dispatch_generation: 4,
            credential_handle_version: None,
            credential_revocation_generation: None,
            exact_origin: "https://example.test".to_string(),
            path_boundary: "/api/".to_string(),
            allowed_destination_origins: ["https://example.test".to_string()].into_iter().collect(),
            max_redirect_hops: 0,
            allow_non_public_destination: false,
            non_public_scope_exception_hash: None,
            expires_at: now + Duration::minutes(5),
        };
        let dispatch = CampaignDispatchAuthoritySnapshotV1 {
            campaign_dispatch_held: false,
            campaign_dispatch_generation: 4,
            operation_admission_held: false,
            operation_admission_generation: 8,
            global_row_version: 12,
            quarantine_pending: false,
        };
        let budgets = HierarchicalBudgetSnapshotV1 {
            operation: head(100),
            wave: head(50),
            campaign: head(20),
            action: head(2),
        };
        let request = NextSendRequestV1 {
            requested_url: "https://example.test/api/resource".to_string(),
            redirect_hop: 0,
            is_retry: false,
            resolved_addresses: vec!["93.184.216.34".parse().unwrap()],
            selected_address: "93.184.216.34".parse().unwrap(),
            budget_delta: [(BudgetAxisV1::Requests, 1)].into_iter().collect(),
            now,
        };
        (authorization, dispatch, budgets, request)
    }

    #[test]
    fn verification_campaign_execution_authorizes_only_a_pinned_validated_send() {
        let (authorization, dispatch, budgets, request) = fixture();
        let send = authorize_next_pinned_send(&authorization, &dispatch, None, &budgets, &request)
            .unwrap();
        assert_eq!(send.pinned_address(), request.selected_address);
        assert_eq!(send.canonical_host(), "example.test");
        assert!(!send.strip_authorization_and_cookies());
    }

    #[test]
    fn verification_campaign_execution_hold_generation_blocks_before_next_send() {
        let (authorization, mut dispatch, budgets, request) = fixture();
        dispatch.campaign_dispatch_held = true;
        assert_eq!(
            authorize_next_pinned_send(&authorization, &dispatch, None, &budgets, &request)
                .unwrap_err(),
            SendAuthorityError::CampaignDispatchHeld
        );
        dispatch.campaign_dispatch_held = false;
        dispatch.campaign_dispatch_generation += 2;
        assert_eq!(
            authorize_next_pinned_send(&authorization, &dispatch, None, &budgets, &request)
                .unwrap_err(),
            SendAuthorityError::CampaignDispatchGenerationDrift
        );
    }

    #[test]
    fn verification_campaign_execution_blocks_while_operation_admission_is_held() {
        let (authorization, mut dispatch, budgets, request) = fixture();
        dispatch.operation_admission_held = true;
        dispatch.operation_admission_generation += 100;
        dispatch.global_row_version += 100;
        assert_eq!(
            authorize_next_pinned_send(&authorization, &dispatch, None, &budgets, &request)
                .unwrap_err(),
            SendAuthorityError::CampaignDispatchHeld
        );
    }

    #[test]
    fn dns_rebinding_or_mixed_public_private_answer_blocks_the_whole_connection() {
        let (authorization, dispatch, budgets, mut request) = fixture();
        request
            .resolved_addresses
            .push("127.0.0.1".parse().unwrap());
        assert_eq!(
            authorize_next_pinned_send(&authorization, &dispatch, None, &budgets, &request)
                .unwrap_err(),
            SendAuthorityError::DnsPolicyDenied
        );
    }

    #[test]
    fn request_budget_governor_blocks_n_plus_one_before_io() {
        let (authorization, dispatch, mut budgets, request) = fixture();
        budgets.action.consumed.insert(BudgetAxisV1::Requests, 2);
        assert_eq!(
            authorize_next_pinned_send(&authorization, &dispatch, None, &budgets, &request)
                .unwrap_err(),
            SendAuthorityError::BudgetExhausted
        );
    }

    #[test]
    fn credential_rotation_or_revocation_blocks_before_secret_injection() {
        let (mut authorization, dispatch, budgets, request) = fixture();
        authorization.credential_handle_version = Some(2);
        authorization.credential_revocation_generation = Some(3);
        let credential = CredentialAuthoritySnapshotV1 {
            handle_version: 3,
            revocation_generation: 3,
            revoked: false,
            injection_origin: authorization.exact_origin.clone(),
        };
        assert_eq!(
            authorize_next_pinned_send(
                &authorization,
                &dispatch,
                Some(&credential),
                &budgets,
                &request,
            )
            .unwrap_err(),
            SendAuthorityError::CredentialDrift
        );
    }

    #[test]
    fn network_destination_policy_rejects_redirect_and_path_widening() {
        let (authorization, dispatch, budgets, mut request) = fixture();
        request.redirect_hop = 1;
        assert_eq!(
            authorize_next_pinned_send(&authorization, &dispatch, None, &budgets, &request)
                .unwrap_err(),
            SendAuthorityError::DestinationPolicyDenied
        );
        request.redirect_hop = 0;
        request.requested_url = "https://example.test/admin".to_string();
        assert_eq!(
            authorize_next_pinned_send(&authorization, &dispatch, None, &budgets, &request)
                .unwrap_err(),
            SendAuthorityError::DestinationPolicyDenied
        );
    }

    #[test]
    fn local_test_fixture_requires_exact_non_public_scope_exception() {
        let (mut authorization, dispatch, budgets, mut request) = fixture();
        authorization.exact_origin = "http://127.0.0.1:8080".to_string();
        authorization.allowed_destination_origins =
            [authorization.exact_origin.clone()].into_iter().collect();
        request.requested_url = "http://127.0.0.1:8080/api/test".to_string();
        request.resolved_addresses = vec!["127.0.0.1".parse().unwrap()];
        request.selected_address = "127.0.0.1".parse().unwrap();
        assert_eq!(
            authorize_next_pinned_send(&authorization, &dispatch, None, &budgets, &request)
                .unwrap_err(),
            SendAuthorityError::DnsPolicyDenied
        );
        authorization.allow_non_public_destination = true;
        authorization.non_public_scope_exception_hash = Some(hash('a'));
        assert!(
            authorize_next_pinned_send(&authorization, &dispatch, None, &budgets, &request).is_ok()
        );
    }

    struct HostRepoFixture {
        context: HostPerHopAuthorityContextV1,
        consumed: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl HostPerHopAuthorityRepository for HostRepoFixture {
        async fn read_current_send_authority(
            &self,
            _selector: PreparedActionSendSelectorV1,
        ) -> Result<HostPerHopAuthorityContextV1, SendAuthorityError> {
            Ok(self.context.clone())
        }

        async fn consume_budget_before_send(
            &self,
            _selector: PreparedActionSendSelectorV1,
            expected_campaign_dispatch_generation: i64,
            expected_budget_fences: [BTreeMap<BudgetAxisV1, i64>; 4],
            _delta: &BTreeMap<BudgetAxisV1, u64>,
        ) -> Result<(), SendAuthorityError> {
            assert_eq!(expected_campaign_dispatch_generation, 4);
            assert!(expected_budget_fences.iter().all(|fences| BudgetAxisV1::ALL
                .iter()
                .all(|axis| fences.get(axis) == Some(&1))));
            self.consumed
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    struct HostResolverFixture(Vec<IpAddr>);

    #[async_trait::async_trait]
    impl HostPinnedResolver for HostResolverFixture {
        async fn resolve_all(
            &self,
            canonical_host: &str,
            canonical_port: u16,
        ) -> Result<Vec<IpAddr>, SendAuthorityError> {
            assert_eq!(canonical_host, "example.test");
            assert_eq!(canonical_port, 443);
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn verification_campaign_execution_host_boundary_owns_clock_dns_and_consume() {
        let (authorization, dispatch, budgets, request) = fixture();
        let repo = HostRepoFixture {
            context: HostPerHopAuthorityContextV1 {
                checked_at: request.now,
                authorization,
                dispatch,
                credential: None,
                budgets,
            },
            consumed: std::sync::atomic::AtomicUsize::new(0),
        };
        let resolver = HostResolverFixture(vec!["93.184.216.34".parse().unwrap()]);
        let selector = PreparedActionSendSelectorV1 {
            operation_id: Uuid::from_u128(1),
            campaign_id: Uuid::from_u128(2),
            prepared_action_id: Uuid::from_u128(3),
            authorization_receipt_id: Uuid::from_u128(4),
            action_execution_id: Uuid::from_u128(5),
        };
        let token = authorize_next_pinned_send_from_host(
            &repo,
            &resolver,
            selector,
            HostNextSendRequestV1 {
                requested_url: request.requested_url,
                redirect_hop: 0,
                is_retry: false,
                budget_delta: request.budget_delta,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            token.pinned_address(),
            "93.184.216.34".parse::<IpAddr>().unwrap()
        );
        assert_eq!(repo.consumed.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
