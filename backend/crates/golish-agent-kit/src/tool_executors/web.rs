use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Arc;

use super::common::{error_result, ToolResult};
use golish_core::WebFetchProvider;
use rig::completion::ToolDefinition;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use url::Url;

/// Execute a web fetch tool using the injected `WebFetchProvider`.
pub async fn execute_web_fetch_tool(
    fetcher: &dyn WebFetchProvider,
    tool_name: &str,
    args: &serde_json::Value,
) -> ToolResult {
    if tool_name != "web_fetch" {
        return error_result(format!("Unknown web fetch tool: {}", tool_name));
    }

    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => {
            return error_result(
                "web_fetch requires a 'url' parameter (string). Example: {\"url\": \"https://example.com\"}"
            )
        }
    };

    match fetcher.fetch(&url).await {
        Ok(result) => (
            json!({
                "url": result.url,
                "content": result.content
            }),
            true,
        ),
        Err(e) => error_result(format!("Failed to fetch {}: {}", url, e)),
    }
}

pub const INTEL_PUBLIC_SEARCH_TOOL: &str = "intel_public_search";
pub const INTEL_PUBLIC_FETCH_TOOL: &str = "intel_public_fetch";
const INTEL_PUBLIC_POLICY_VIOLATION: &str = "intel_public_policy_violation";
const MAX_REDIRECT_HOPS: usize = 3;
const MAX_BODY_BYTES: usize = 128 * 1024;
const MAX_SEARCH_HITS: usize = 20;

pub fn intel_public_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: INTEL_PUBLIC_SEARCH_TOOL.to_string(),
            description: "Fixture/dev-only host-owned passive public evidence search. Results are untrusted data and become visible only after redacted evidence and an audit receipt persist. Provider server-side search is forbidden.".to_string(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "query": { "type": "string", "minLength": 1, "maxLength": 512 }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: INTEL_PUBLIC_FETCH_TOOL.to_string(),
            description: "Fixture/dev-only host-owned strict-passive public URL fetch. GET/HEAD only; all DNS answers and redirects are revalidated and the validated connect address is pinned before evidence-first visibility.".to_string(),
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "url": { "type": "string", "minLength": 1, "maxLength": 2048 },
                    "method": { "type": "string", "enum": ["GET", "HEAD"] }
                },
                "required": ["url"]
            }),
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntelPublicCapabilityMode {
    StrictPassiveFixture,
    PublicWebReadonlyDisabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelPublicSearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedIntelPublicRequest {
    pub method: String,
    pub url: String,
    pub host: String,
    pub pinned_address: IpAddr,
    /// Strict-passive transport never forwards credentials, cookies or model
    /// headers. The fake transport can assert this remains empty.
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntelPublicTransportResponse {
    pub final_url: String,
    pub status: u16,
    pub mime: String,
    pub body: Vec<u8>,
    pub redirect_location: Option<String>,
    pub connected_address: IpAddr,
    pub timestamp_ms: i64,
}

#[async_trait::async_trait]
pub trait IntelPublicFixtureTransport: Send + Sync {
    fn is_fake(&self) -> bool;
    async fn search(&self, query: &str) -> Result<Vec<IntelPublicSearchHit>, String>;
    async fn resolve_all(&self, host: &str) -> Result<Vec<IpAddr>, String>;
    async fn fetch_pinned(
        &self,
        request: &PinnedIntelPublicRequest,
    ) -> Result<IntelPublicTransportResponse, String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelPublicEvidenceReceipt {
    pub kind: String,
    pub tool_name: String,
    pub method: String,
    pub input: String,
    pub final_url: Option<String>,
    pub redirect_chain: Vec<String>,
    pub status: String,
    pub http_status: Option<u16>,
    pub timestamp_ms: i64,
    pub mime: Option<String>,
    pub bounded_body: String,
    pub content_sha256: String,
    pub pinned_addresses: Vec<String>,
    pub capability: String,
    pub reason: Option<String>,
}

#[async_trait::async_trait]
pub trait IntelPublicEvidenceSink: Send + Sync {
    async fn append_evidence(&self, receipt: &IntelPublicEvidenceReceipt)
        -> Result<String, String>;
    async fn append_audit_receipt(
        &self,
        receipt: &IntelPublicEvidenceReceipt,
        evidence_ref: &str,
    ) -> Result<String, String>;
}

#[async_trait::async_trait]
pub trait IntelPublicEvidenceAdapter: Send + Sync {
    async fn execute(&self, tool_name: &str, args: &serde_json::Value) -> ToolResult;
}

pub struct StrictPassiveIntelPublicAdapter {
    mode: IntelPublicCapabilityMode,
    transport: Arc<dyn IntelPublicFixtureTransport>,
    evidence: Arc<dyn IntelPublicEvidenceSink>,
    target_owned_hosts: BTreeSet<String>,
}

impl StrictPassiveIntelPublicAdapter {
    pub fn new_fixture(
        mode: IntelPublicCapabilityMode,
        transport: Arc<dyn IntelPublicFixtureTransport>,
        evidence: Arc<dyn IntelPublicEvidenceSink>,
        target_owned_hosts: impl IntoIterator<Item = String>,
    ) -> Result<Self, &'static str> {
        if !transport.is_fake() {
            return Err("INTEL_PUBLIC_REAL_TRANSPORT_FORBIDDEN_IN_PLAN_A");
        }
        Ok(Self {
            mode,
            transport,
            evidence,
            target_owned_hosts: target_owned_hosts
                .into_iter()
                .map(|host| host.trim().trim_end_matches('.').to_ascii_lowercase())
                .filter(|host| !host.is_empty())
                .collect(),
        })
    }

    async fn execute_strict(&self, tool_name: &str, args: &serde_json::Value) -> ToolResult {
        if self.mode == IntelPublicCapabilityMode::PublicWebReadonlyDisabled {
            return self
                .persist_unsupported(tool_name, "public_web_readonly", "disabled_not_in_plan_a")
                .await;
        }
        match tool_name {
            INTEL_PUBLIC_SEARCH_TOOL => self.execute_search(args).await,
            INTEL_PUBLIC_FETCH_TOOL => self.execute_fetch(args).await,
            INTEL_PUBLIC_POLICY_VIOLATION => {
                self.persist_blocked(
                    tool_name,
                    "provider_server_web_search",
                    "provider_server_search_bypasses_host_evidence_adapter",
                )
                .await
            }
            _ => error_result(format!("Unknown Intel public evidence tool: {tool_name}")),
        }
    }

    async fn execute_search(&self, args: &serde_json::Value) -> ToolResult {
        let Some(query) = args
            .get("query")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|query| bounded_text(query, 512))
        else {
            return error_result("intel_public_search requires a bounded 'query' string");
        };
        let mut hits = match self.transport.search(query).await {
            Ok(hits) => hits,
            Err(reason) => {
                return error_result(format!("intel public fake search failed: {reason}"))
            }
        };
        hits.truncate(MAX_SEARCH_HITS);
        let mut pinned_addresses = Vec::new();
        for hit in &hits {
            if !bounded_text(&hit.title, 512)
                || !bounded_text(&hit.url, 2_048)
                || !bounded_text(&hit.snippet, 2_048)
            {
                return error_result("INTEL_PUBLIC_SEARCH_RESULT_BOUNDS_REJECTED");
            }
            let validated = match self.validate_url(&hit.url).await {
                Ok(validated) => validated,
                Err(reason) => return error_result(reason),
            };
            pinned_addresses.push(validated.pinned_address.to_string());
        }
        let body = serde_json::to_vec(&hits).unwrap_or_default();
        if body.len() > MAX_BODY_BYTES {
            return error_result("INTEL_PUBLIC_BODY_TOO_LARGE");
        }
        let receipt = IntelPublicEvidenceReceipt {
            kind: "intel.public_evidence_receipt.v1".to_string(),
            tool_name: INTEL_PUBLIC_SEARCH_TOOL.to_string(),
            method: "SEARCH".to_string(),
            input: query.to_string(),
            final_url: None,
            redirect_chain: Vec::new(),
            status: if hits.is_empty() {
                "empty"
            } else {
                "succeeded"
            }
            .to_string(),
            http_status: None,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            mime: Some("application/json".to_string()),
            bounded_body: String::from_utf8_lossy(&body).into_owned(),
            content_sha256: sha256_hex(&body),
            pinned_addresses,
            capability: "strict_passive".to_string(),
            reason: None,
        };
        self.persist_then_expose(receipt, json!({ "hits": hits }))
            .await
    }

    async fn execute_fetch(&self, args: &serde_json::Value) -> ToolResult {
        let Some(input_url) = args.get("url").and_then(serde_json::Value::as_str) else {
            return error_result("intel_public_fetch requires a 'url' string");
        };
        if !bounded_text(input_url, 2_048) {
            return error_result("INTEL_PUBLIC_URL_INVALID");
        }
        let method = args
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("GET")
            .to_ascii_uppercase();
        if !matches!(method.as_str(), "GET" | "HEAD") {
            return error_result("intel_public_fetch permits only GET or HEAD");
        }

        let mut current_url = input_url.to_string();
        let mut redirect_chain = Vec::new();
        let mut pinned_addresses = Vec::new();
        for hop in 0..=MAX_REDIRECT_HOPS {
            let validated = match self.validate_url(&current_url).await {
                Ok(validated) => validated,
                Err(reason) => return error_result(reason),
            };
            pinned_addresses.push(validated.pinned_address.to_string());
            let response = match self
                .transport
                .fetch_pinned(&PinnedIntelPublicRequest {
                    method: method.clone(),
                    url: validated.url.to_string(),
                    host: validated.host,
                    pinned_address: validated.pinned_address,
                    headers: Vec::new(),
                })
                .await
            {
                Ok(response) => response,
                Err(reason) => {
                    return error_result(format!("intel public fake fetch failed: {reason}"));
                }
            };
            if response.connected_address != validated.pinned_address {
                return error_result("INTEL_PUBLIC_DNS_REBINDING_REJECTED");
            }
            if !(100..=599).contains(&response.status) || response.timestamp_ms <= 0 {
                return error_result("INTEL_PUBLIC_RESPONSE_METADATA_INVALID");
            }
            if let Some(location) = response.redirect_location.as_deref() {
                if hop == MAX_REDIRECT_HOPS {
                    return error_result("INTEL_PUBLIC_REDIRECT_LIMIT_EXCEEDED");
                }
                let next = match validated.url.join(location) {
                    Ok(next) => next,
                    Err(_) => return error_result("INTEL_PUBLIC_REDIRECT_URL_INVALID"),
                };
                redirect_chain.push(next.to_string());
                current_url = next.to_string();
                continue;
            }
            if response.final_url != validated.url.as_str() {
                return error_result("INTEL_PUBLIC_UNDECLARED_FINAL_URL_REJECTED");
            }
            if response.body.len() > MAX_BODY_BYTES {
                return error_result("INTEL_PUBLIC_BODY_TOO_LARGE");
            }
            let mime = response
                .mime
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if method != "HEAD"
                && !(mime.starts_with("text/")
                    || matches!(
                        mime.as_str(),
                        "application/json" | "application/xml" | "application/xhtml+xml"
                    ))
            {
                return error_result("INTEL_PUBLIC_FILE_DOWNLOAD_REJECTED");
            }
            let bounded_body = String::from_utf8_lossy(&response.body).into_owned();
            let receipt = IntelPublicEvidenceReceipt {
                kind: "intel.public_evidence_receipt.v1".to_string(),
                tool_name: INTEL_PUBLIC_FETCH_TOOL.to_string(),
                method,
                input: input_url.to_string(),
                final_url: Some(response.final_url.clone()),
                redirect_chain,
                status: "succeeded".to_string(),
                http_status: Some(response.status),
                timestamp_ms: response.timestamp_ms,
                mime: Some(response.mime.clone()),
                bounded_body: bounded_body.clone(),
                content_sha256: sha256_hex(&response.body),
                pinned_addresses,
                capability: "strict_passive".to_string(),
                reason: None,
            };
            return self
                .persist_then_expose(
                    receipt,
                    json!({
                        "url": response.final_url,
                        "status": response.status,
                        "mime": response.mime,
                        "content": bounded_body,
                    }),
                )
                .await;
        }
        error_result("INTEL_PUBLIC_REDIRECT_LIMIT_EXCEEDED")
    }

    async fn validate_url(&self, raw: &str) -> Result<ValidatedPublicUrl, String> {
        let url = Url::parse(raw).map_err(|_| "INTEL_PUBLIC_URL_INVALID".to_string())?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("INTEL_PUBLIC_SCHEME_FORBIDDEN".to_string());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("INTEL_PUBLIC_USERINFO_FORBIDDEN".to_string());
        }
        let host = url
            .host_str()
            .ok_or_else(|| "INTEL_PUBLIC_HOST_MISSING".to_string())?
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if self.target_owned_hosts.iter().any(|target| {
            host == *target
                || host
                    .strip_suffix(target)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        }) {
            return Err("INTEL_PUBLIC_TARGET_OWNED_HOST_REJECTED".to_string());
        }
        let addresses = match host.parse::<IpAddr>() {
            Ok(address) => vec![address],
            Err(_) => self
                .transport
                .resolve_all(&host)
                .await
                .map_err(|reason| format!("INTEL_PUBLIC_DNS_FAILED:{reason}"))?,
        };
        if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(*address)) {
            return Err("INTEL_PUBLIC_NON_PUBLIC_ADDRESS_REJECTED".to_string());
        }
        Ok(ValidatedPublicUrl {
            url,
            host,
            pinned_address: addresses[0],
        })
    }

    async fn persist_then_expose(
        &self,
        receipt: IntelPublicEvidenceReceipt,
        visible_untrusted_data: serde_json::Value,
    ) -> ToolResult {
        let evidence_ref = match self.evidence.append_evidence(&receipt).await {
            Ok(reference) => reference,
            Err(_) => return error_result("INTEL_PUBLIC_EVIDENCE_PERSISTENCE_FAILED"),
        };
        let audit_ref = match self
            .evidence
            .append_audit_receipt(&receipt, &evidence_ref)
            .await
        {
            Ok(reference) => reference,
            Err(_) => return error_result("INTEL_PUBLIC_RECEIPT_PERSISTENCE_FAILED"),
        };
        (
            json!({
                "untrusted_data": visible_untrusted_data,
                "evidence_ref": evidence_ref,
                "audit_receipt_ref": audit_ref,
                "content_sha256": receipt.content_sha256,
            }),
            true,
        )
    }

    async fn persist_unsupported(
        &self,
        tool_name: &str,
        capability: &str,
        reason: &str,
    ) -> ToolResult {
        self.persist_terminal(tool_name, "unsupported", capability, reason)
            .await
    }

    async fn persist_blocked(&self, tool_name: &str, capability: &str, reason: &str) -> ToolResult {
        self.persist_terminal(tool_name, "blocked", capability, reason)
            .await
    }

    async fn persist_terminal(
        &self,
        tool_name: &str,
        status: &str,
        capability: &str,
        reason: &str,
    ) -> ToolResult {
        let body = json!({ "status": status, "capability": capability, "reason": reason });
        let bytes = serde_json::to_vec(&body).unwrap_or_default();
        let receipt = IntelPublicEvidenceReceipt {
            kind: "intel.public_evidence_receipt.v1".to_string(),
            tool_name: tool_name.to_string(),
            method: "NONE".to_string(),
            input: String::new(),
            final_url: None,
            redirect_chain: Vec::new(),
            status: status.to_string(),
            http_status: None,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            mime: Some("application/json".to_string()),
            bounded_body: body.to_string(),
            content_sha256: sha256_hex(&bytes),
            pinned_addresses: Vec::new(),
            capability: capability.to_string(),
            reason: Some(reason.to_string()),
        };
        let evidence_ref = match self.evidence.append_evidence(&receipt).await {
            Ok(reference) => reference,
            Err(_) => return error_result("INTEL_PUBLIC_EVIDENCE_PERSISTENCE_FAILED"),
        };
        if self
            .evidence
            .append_audit_receipt(&receipt, &evidence_ref)
            .await
            .is_err()
        {
            return error_result("INTEL_PUBLIC_RECEIPT_PERSISTENCE_FAILED");
        }
        (body, false)
    }
}

#[async_trait::async_trait]
impl IntelPublicEvidenceAdapter for StrictPassiveIntelPublicAdapter {
    async fn execute(&self, tool_name: &str, args: &serde_json::Value) -> ToolResult {
        self.execute_strict(tool_name, args).await
    }
}

pub async fn execute_intel_public_tool(
    adapter: &dyn IntelPublicEvidenceAdapter,
    tool_name: &str,
    args: &serde_json::Value,
) -> ToolResult {
    adapter.execute(tool_name, args).await
}

pub async fn record_intel_public_policy_violation(
    adapter: &dyn IntelPublicEvidenceAdapter,
    args: &serde_json::Value,
) -> ToolResult {
    adapter.execute(INTEL_PUBLIC_POLICY_VIOLATION, args).await
}

struct ValidatedPublicUrl {
    url: Url,
    host: String,
    pinned_address: IpAddr,
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            !matches!(
                octets,
                [0, ..]
                    | [10, ..]
                    | [100, 64..=127, ..]
                    | [127, ..]
                    | [169, 254, ..]
                    | [172, 16..=31, ..]
                    | [192, 0, 0, ..]
                    | [192, 0, 2, ..]
                    | [192, 88, 99, ..]
                    | [192, 168, ..]
                    | [198, 18..=19, ..]
                    | [198, 51, 100, ..]
                    | [203, 0, 113, ..]
                    | [224..=255, ..]
            )
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            (segments[0] & 0xe000) == 0x2000
                && !(segments[0] == 0x2001
                    && (segments[1] == 0x0000
                        || segments[1] == 0x0db8
                        || segments[1] == 0x0002
                        || (segments[1] & 0xfff0) == 0x0010))
                && (segments[0] != 0x2002 || is_public_6to4_embedded(segments))
                && !is_ipv4_mapped_non_public(address)
        }
    }
}

fn is_ipv4_mapped_non_public(address: Ipv6Addr) -> bool {
    address
        .to_ipv4_mapped()
        .is_some_and(|mapped| !is_public_ip(IpAddr::V4(mapped)))
}

fn is_public_6to4_embedded(segments: [u16; 8]) -> bool {
    let high = segments[1].to_be_bytes();
    let low = segments[2].to_be_bytes();
    is_public_ip(IpAddr::V4(std::net::Ipv4Addr::new(
        high[0], high[1], low[0], low[1],
    )))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bounded_text(value: &str, max_chars: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= max_chars
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod intel_public_tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    use super::*;

    struct FakeTransport {
        resolutions: Mutex<HashMap<String, Vec<IpAddr>>>,
        responses: Mutex<VecDeque<IntelPublicTransportResponse>>,
        hits: Vec<IntelPublicSearchHit>,
    }

    #[async_trait::async_trait]
    impl IntelPublicFixtureTransport for FakeTransport {
        fn is_fake(&self) -> bool {
            true
        }

        async fn search(&self, _query: &str) -> Result<Vec<IntelPublicSearchHit>, String> {
            Ok(self.hits.clone())
        }

        async fn resolve_all(&self, host: &str) -> Result<Vec<IpAddr>, String> {
            self.resolutions
                .lock()
                .unwrap()
                .get(host)
                .cloned()
                .ok_or_else(|| format!("no fixture DNS for {host}"))
        }

        async fn fetch_pinned(
            &self,
            _request: &PinnedIntelPublicRequest,
        ) -> Result<IntelPublicTransportResponse, String> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| "no fixture response".to_string())
        }
    }

    #[derive(Default)]
    struct FakeEvidence {
        receipts: Mutex<Vec<IntelPublicEvidenceReceipt>>,
        fail_evidence: bool,
    }

    #[async_trait::async_trait]
    impl IntelPublicEvidenceSink for FakeEvidence {
        async fn append_evidence(
            &self,
            receipt: &IntelPublicEvidenceReceipt,
        ) -> Result<String, String> {
            if self.fail_evidence {
                return Err("fixture evidence failure".to_string());
            }
            self.receipts.lock().unwrap().push(receipt.clone());
            Ok("evidence:fixture".to_string())
        }

        async fn append_audit_receipt(
            &self,
            _receipt: &IntelPublicEvidenceReceipt,
            _evidence_ref: &str,
        ) -> Result<String, String> {
            Ok("audit:fixture".to_string())
        }
    }

    fn public_ip() -> IpAddr {
        "93.184.216.34".parse().unwrap()
    }

    fn response(
        url: &str,
        connected_address: IpAddr,
        body: &str,
        redirect_location: Option<&str>,
    ) -> IntelPublicTransportResponse {
        IntelPublicTransportResponse {
            final_url: url.to_string(),
            status: if redirect_location.is_some() {
                302
            } else {
                200
            },
            mime: "text/plain".to_string(),
            body: body.as_bytes().to_vec(),
            redirect_location: redirect_location.map(str::to_string),
            connected_address,
            timestamp_ms: 1,
        }
    }

    fn adapter(
        mode: IntelPublicCapabilityMode,
        resolutions: HashMap<String, Vec<IpAddr>>,
        responses: Vec<IntelPublicTransportResponse>,
        evidence: Arc<FakeEvidence>,
    ) -> StrictPassiveIntelPublicAdapter {
        StrictPassiveIntelPublicAdapter::new_fixture(
            mode,
            Arc::new(FakeTransport {
                resolutions: Mutex::new(resolutions),
                responses: Mutex::new(responses.into()),
                hits: Vec::new(),
            }),
            evidence,
            ["target.example".to_string()],
        )
        .unwrap()
    }

    #[test]
    fn strict_public_address_classifier_rejects_special_use_ranges() {
        for address in [
            "0.1.2.3",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "192.0.0.1",
            "198.18.0.1",
            "240.0.0.1",
            "2001:db8::1",
            "2001::1",
            "2002:7f00:1::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert!(!is_public_ip(address.parse().unwrap()), "{address}");
        }
        assert!(is_public_ip(public_ip()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[tokio::test]
    async fn result_is_not_model_visible_when_evidence_append_fails() {
        let evidence = Arc::new(FakeEvidence {
            fail_evidence: true,
            ..FakeEvidence::default()
        });
        let adapter = adapter(
            IntelPublicCapabilityMode::StrictPassiveFixture,
            [("public.example".to_string(), vec![public_ip()])]
                .into_iter()
                .collect(),
            vec![response(
                "https://public.example/",
                public_ip(),
                "provider-secret-body",
                None,
            )],
            evidence,
        );
        let (value, success) = adapter
            .execute(
                INTEL_PUBLIC_FETCH_TOOL,
                &json!({"url": "https://public.example/"}),
            )
            .await;
        assert!(!success);
        assert!(!value.to_string().contains("provider-secret-body"));
    }

    #[tokio::test]
    async fn fetch_revalidates_every_redirect_and_all_resolved_addresses() {
        let evidence = Arc::new(FakeEvidence::default());
        let adapter = adapter(
            IntelPublicCapabilityMode::StrictPassiveFixture,
            [
                ("public.example".to_string(), vec![public_ip()]),
                (
                    "mixed.example".to_string(),
                    vec![public_ip(), "127.0.0.1".parse().unwrap()],
                ),
            ]
            .into_iter()
            .collect(),
            vec![response(
                "https://public.example/",
                public_ip(),
                "",
                Some("https://mixed.example/private"),
            )],
            evidence,
        );
        let (value, success) = adapter
            .execute(
                INTEL_PUBLIC_FETCH_TOOL,
                &json!({"url": "https://public.example/"}),
            )
            .await;
        assert!(!success);
        assert!(value
            .to_string()
            .contains("INTEL_PUBLIC_NON_PUBLIC_ADDRESS_REJECTED"));
    }

    #[tokio::test]
    async fn dns_rebinding_after_validation_is_rejected() {
        let evidence = Arc::new(FakeEvidence::default());
        let adapter = adapter(
            IntelPublicCapabilityMode::StrictPassiveFixture,
            [("public.example".to_string(), vec![public_ip()])]
                .into_iter()
                .collect(),
            vec![response(
                "https://public.example/",
                "1.1.1.1".parse().unwrap(),
                "must-not-return",
                None,
            )],
            evidence,
        );
        let (value, success) = adapter
            .execute(
                INTEL_PUBLIC_FETCH_TOOL,
                &json!({"url": "https://public.example/"}),
            )
            .await;
        assert!(!success);
        assert!(value
            .to_string()
            .contains("INTEL_PUBLIC_DNS_REBINDING_REJECTED"));
        assert!(!value.to_string().contains("must-not-return"));
    }

    #[tokio::test]
    async fn public_web_readonly_is_explicitly_disabled_in_plan_a() {
        let evidence = Arc::new(FakeEvidence::default());
        let adapter = adapter(
            IntelPublicCapabilityMode::PublicWebReadonlyDisabled,
            HashMap::new(),
            Vec::new(),
            evidence.clone(),
        );
        let (value, success) = adapter
            .execute(INTEL_PUBLIC_SEARCH_TOOL, &json!({"query": "Acme"}))
            .await;
        assert!(!success);
        assert_eq!(value["status"], "unsupported");
        assert_eq!(value["capability"], "public_web_readonly");
        assert_eq!(evidence.receipts.lock().unwrap().len(), 1);
    }
}
