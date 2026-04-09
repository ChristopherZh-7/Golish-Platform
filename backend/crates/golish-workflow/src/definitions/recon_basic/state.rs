use serde::{Deserialize, Serialize};

pub const STATE_KEY: &str = "recon_state";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReconStage {
    Initialized,
    ToolCheck,
    DnsLookup,
    HttpProbe,
    PortScan,
    TechFingerprint,
    SubdomainEnum,
    Recording,
    Analysis,
    Completed,
}

impl Default for ReconStage {
    fn default() -> Self {
        Self::Initialized
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PortInfo {
    pub port: u16,
    pub protocol: String,
    pub service: String,
    pub version: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TargetReconData {
    pub value: String,
    pub ips: Vec<String>,
    pub http_status: Option<u16>,
    pub http_server: String,
    pub http_redirect: String,
    pub ports: Vec<PortInfo>,
    pub technologies: Vec<String>,
    pub subdomains: Vec<String>,
    pub raw_outputs: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvailableTools {
    pub dig: bool,
    pub curl: bool,
    pub nmap: bool,
    pub subfinder: bool,
    pub httpx: bool,
    pub whatweb: bool,
    pub nc: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReconState {
    pub targets: Vec<String>,
    pub project_path: String,
    pub project_name: String,
    pub tools: AvailableTools,
    pub results: Vec<TargetReconData>,
    pub stage: ReconStage,
    pub errors: Vec<String>,
    pub summary: Option<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
}
