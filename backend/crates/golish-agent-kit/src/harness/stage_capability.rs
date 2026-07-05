//! Stage-local capability registry.
//!
//! A capability is the business action the agent should choose (for example
//! `eas.fingerprint_services`). The concrete tools remain implementation
//! details guarded by the stage tool taxonomy.

use serde::{Deserialize, Serialize};

use super::types::StageKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityRisk {
    Passive,
    Active,
    Exploit,
    PostExploit,
}

impl CapabilityRisk {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passive => "passive",
            Self::Active => "active",
            Self::Exploit => "exploit",
            Self::PostExploit => "post_exploit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityRunnerKind {
    MetadataOnly,
    ExistingDirectTool,
    PentestRunRecipe,
    BackendWrapper,
}

impl CapabilityRunnerKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MetadataOnly => "metadata_only",
            Self::ExistingDirectTool => "existing_direct_tool",
            Self::PentestRunRecipe => "pentest_run_recipe",
            Self::BackendWrapper => "backend_wrapper",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageCapabilitySpec {
    pub id: &'static str,
    pub label: &'static str,
    pub stage: StageKind,
    pub techniques: &'static [&'static str],
    pub tool_names: &'static [&'static str],
    pub allowed_tool_types: &'static [&'static str],
    pub risk: CapabilityRisk,
    pub batchable: bool,
    pub max_batch: usize,
    pub writes: &'static [&'static str],
    pub runner: CapabilityRunnerKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageCapabilitySuggestion {
    pub id: String,
    pub label: String,
    pub tools: Vec<String>,
    pub risk: String,
    pub batchable: bool,
    pub max_batch: usize,
    pub reason: String,
}

impl StageCapabilitySpec {
    pub fn suggestion(&self, technique: Option<&str>) -> StageCapabilitySuggestion {
        let reason = match technique {
            Some(technique) => format!("closes {technique} via {}", self.id),
            None => format!("stage-local capability for {}", self.stage.as_str()),
        };
        StageCapabilitySuggestion {
            id: self.id.to_string(),
            label: self.label.to_string(),
            tools: self
                .tool_names
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
            risk: self.risk.as_str().to_string(),
            batchable: self.batchable,
            max_batch: self.max_batch,
            reason,
        }
    }
}

const CAPABILITIES: &[StageCapabilitySpec] = &[
    StageCapabilitySpec {
        id: "scope.resolve_company",
        label: "Resolve company",
        stage: StageKind::Scoping,
        techniques: &[],
        tool_names: &["recon_lookup_company"],
        allowed_tool_types: &[],
        risk: CapabilityRisk::Passive,
        batchable: false,
        max_batch: 1,
        writes: &["organizations", "scope_review"],
        runner: CapabilityRunnerKind::ExistingDirectTool,
    },
    StageCapabilitySpec {
        id: "scope.discover_subsidiaries",
        label: "Discover subsidiaries",
        stage: StageKind::Scoping,
        techniques: &["GOLISH-INTEL-SUBSIDIARY"],
        tool_names: &["recon_discover_subsidiaries"],
        allowed_tool_types: &[],
        risk: CapabilityRisk::Passive,
        batchable: false,
        max_batch: 1,
        writes: &["organizations", "unit_review"],
        runner: CapabilityRunnerKind::ExistingDirectTool,
    },
    StageCapabilitySpec {
        id: "scope.confirm_scope",
        label: "Confirm scope",
        stage: StageKind::Scoping,
        techniques: &[],
        tool_names: &["ask_human", "manage_organizations"],
        allowed_tool_types: &[],
        risk: CapabilityRisk::Passive,
        batchable: false,
        max_batch: 1,
        writes: &["scope_decision"],
        runner: CapabilityRunnerKind::ExistingDirectTool,
    },
    StageCapabilitySpec {
        id: "intel.collect_passive_assets",
        label: "Collect passive assets",
        stage: StageKind::TargetIntel,
        techniques: &[
            "GOLISH-INTEL-DNS",
            "GOLISH-INTEL-ASN",
            "GOLISH-INTEL-CT",
            "GOLISH-INTEL-SUBDOMAIN",
            "GOLISH-INTEL-OSINT",
        ],
        tool_names: &["recon_map_assets"],
        allowed_tool_types: &[],
        risk: CapabilityRisk::Passive,
        batchable: true,
        max_batch: 25,
        writes: &[
            "targets",
            "dns_records",
            "source_query_log",
            "technique_outcomes",
        ],
        runner: CapabilityRunnerKind::ExistingDirectTool,
    },
    StageCapabilitySpec {
        id: "intel.collect_whois",
        label: "Collect WHOIS/RDAP",
        stage: StageKind::TargetIntel,
        techniques: &["GOLISH-INTEL-WHOIS"],
        tool_names: &["recon_lookup_whois"],
        allowed_tool_types: &[],
        risk: CapabilityRisk::Passive,
        batchable: true,
        max_batch: 25,
        writes: &["organizations", "source_query_log", "technique_outcomes"],
        runner: CapabilityRunnerKind::ExistingDirectTool,
    },
    StageCapabilitySpec {
        id: "intel.record_terminal_gap",
        label: "Record terminal intel gap",
        stage: StageKind::TargetIntel,
        techniques: &[
            "GOLISH-INTEL-DNS",
            "GOLISH-INTEL-WHOIS",
            "GOLISH-INTEL-ASN",
            "GOLISH-INTEL-CT",
            "GOLISH-INTEL-SUBDOMAIN",
            "GOLISH-INTEL-OSINT",
        ],
        tool_names: &[],
        allowed_tool_types: &[],
        risk: CapabilityRisk::Passive,
        batchable: true,
        max_batch: 25,
        writes: &["coverage_terminal_cell"],
        runner: CapabilityRunnerKind::MetadataOnly,
    },
    StageCapabilitySpec {
        id: "eas.probe_http_liveness",
        label: "Probe HTTP liveness",
        stage: StageKind::ExternalAttackSurface,
        techniques: &["GOLISH-EAS-LIVENESS"],
        tool_names: &["eas_probe_http_liveness"],
        allowed_tool_types: &["recon/http", "recon/port-scan"],
        risk: CapabilityRisk::Active,
        batchable: true,
        max_batch: 100,
        writes: &["targets.http_status", "web_origins", "technique_outcomes"],
        runner: CapabilityRunnerKind::BackendWrapper,
    },
    StageCapabilitySpec {
        id: "eas.discover_ports",
        label: "Discover ports",
        stage: StageKind::ExternalAttackSurface,
        techniques: &["GOLISH-EAS-PORT"],
        tool_names: &["eas_discover_ports"],
        allowed_tool_types: &["recon/port-scan"],
        risk: CapabilityRisk::Active,
        batchable: true,
        max_batch: 256,
        writes: &["targets.ports", "network_endpoints", "technique_outcomes"],
        runner: CapabilityRunnerKind::BackendWrapper,
    },
    StageCapabilitySpec {
        id: "eas.fingerprint_services",
        label: "Fingerprint services",
        stage: StageKind::ExternalAttackSurface,
        techniques: &["GOLISH-EAS-SERVICE-FINGERPRINT"],
        tool_names: &["eas_fingerprint_services"],
        allowed_tool_types: &["recon/port-scan"],
        risk: CapabilityRisk::Active,
        batchable: true,
        max_batch: 128,
        writes: &["fingerprints", "network_endpoints", "technique_outcomes"],
        runner: CapabilityRunnerKind::BackendWrapper,
    },
    StageCapabilitySpec {
        id: "eas.capture_web_screenshot",
        label: "Capture web screenshot",
        stage: StageKind::ExternalAttackSurface,
        techniques: &[],
        tool_names: &["gowitness"],
        allowed_tool_types: &["recon/visual"],
        risk: CapabilityRisk::Active,
        batchable: true,
        max_batch: 50,
        writes: &["artifacts", "audit_log"],
        runner: CapabilityRunnerKind::PentestRunRecipe,
    },
    StageCapabilitySpec {
        id: "eas.record_terminal_gap",
        label: "Record terminal EAS gap",
        stage: StageKind::ExternalAttackSurface,
        techniques: &[
            "GOLISH-EAS-LIVENESS",
            "GOLISH-EAS-PORT",
            "GOLISH-EAS-SERVICE-FINGERPRINT",
        ],
        tool_names: &[],
        allowed_tool_types: &[],
        risk: CapabilityRisk::Active,
        batchable: true,
        max_batch: 100,
        writes: &["coverage_terminal_cell"],
        runner: CapabilityRunnerKind::MetadataOnly,
    },
    StageCapabilitySpec {
        id: "enum.collect_browser_surface",
        label: "Collect browser surface",
        stage: StageKind::Enumeration,
        techniques: &["GOLISH-ENUM-JS", "GOLISH-ENUM-PARAM"],
        tool_names: &["browser_collect_js_api"],
        allowed_tool_types: &["recon/crawler"],
        risk: CapabilityRisk::Active,
        batchable: true,
        max_batch: 50,
        writes: &["js_analysis_results", "api_endpoints", "technique_outcomes"],
        runner: CapabilityRunnerKind::ExistingDirectTool,
    },
    StageCapabilitySpec {
        id: "enum.extract_js_apis",
        label: "Extract JS APIs",
        stage: StageKind::Enumeration,
        techniques: &["GOLISH-ENUM-JSAPI", "GOLISH-ENUM-PARAM"],
        tool_names: &["js_extract_apis"],
        allowed_tool_types: &["recon/crawler"],
        risk: CapabilityRisk::Active,
        batchable: true,
        max_batch: 50,
        writes: &["api_endpoints", "technique_outcomes"],
        runner: CapabilityRunnerKind::ExistingDirectTool,
    },
    StageCapabilitySpec {
        id: "enum.probe_routes",
        label: "Probe routes",
        stage: StageKind::Enumeration,
        techniques: &["GOLISH-ENUM-DIR"],
        tool_names: &["route_probe_paths"],
        allowed_tool_types: &["web/route-probe"],
        risk: CapabilityRisk::Active,
        batchable: true,
        max_batch: 50,
        writes: &["directory_entries", "technique_outcomes"],
        runner: CapabilityRunnerKind::ExistingDirectTool,
    },
    StageCapabilitySpec {
        id: "enum.crawl_same_origin_urls",
        label: "Crawl same-origin URLs",
        stage: StageKind::Enumeration,
        techniques: &[],
        tool_names: &["enum_crawl_same_origin_urls"],
        allowed_tool_types: &["recon/crawler"],
        risk: CapabilityRisk::Active,
        batchable: true,
        max_batch: 50,
        writes: &["crawl_observations", "api_endpoints"],
        runner: CapabilityRunnerKind::BackendWrapper,
    },
    StageCapabilitySpec {
        id: "enum.record_terminal_gap",
        label: "Record terminal enumeration gap",
        stage: StageKind::Enumeration,
        techniques: &[
            "GOLISH-ENUM-JS",
            "GOLISH-ENUM-DIR",
            "GOLISH-ENUM-PARAM",
            "GOLISH-ENUM-JSAPI",
        ],
        tool_names: &[],
        allowed_tool_types: &[],
        risk: CapabilityRisk::Active,
        batchable: true,
        max_batch: 50,
        writes: &["coverage_terminal_cell"],
        runner: CapabilityRunnerKind::MetadataOnly,
    },
    StageCapabilitySpec {
        id: "vuln.run_formulaic_sweep",
        label: "Run formulaic vulnerability sweep",
        stage: StageKind::VulnTriage,
        techniques: &[
            "WSTG-INPV-05",
            "WSTG-INPV-01",
            "WSTG-INPV-12",
            "WSTG-ATHZ-04",
            "WSTG-ATHN-02",
            "WSTG-SESS-02",
            "WSTG-CONF-05",
            "WSTG-CRYP-03",
            "WSTG-INFO",
            "GOLISH-NDAY",
        ],
        tool_names: &["vuln_run_formulaic_sweep"],
        allowed_tool_types: &["vuln_run_formulaic_sweep"],
        risk: CapabilityRisk::Active,
        batchable: true,
        max_batch: 50,
        writes: &["findings", "audit_log", "technique_outcomes"],
        runner: CapabilityRunnerKind::BackendWrapper,
    },
    StageCapabilitySpec {
        id: "attack.synthesize_candidates",
        label: "Synthesize attack candidates",
        stage: StageKind::AttackCandidate,
        techniques: &[],
        tool_names: &["query_target_data", "list_recent_evidence"],
        allowed_tool_types: &[],
        risk: CapabilityRisk::Passive,
        batchable: false,
        max_batch: 1,
        writes: &["attack_candidates"],
        runner: CapabilityRunnerKind::MetadataOnly,
    },
    StageCapabilitySpec {
        id: "verify.validate_candidate",
        label: "Validate approved candidate",
        stage: StageKind::Verification,
        techniques: &[],
        tool_names: &["sqlmap", "metasploit", "searchsploit"],
        allowed_tool_types: &["web/injection", "exploit"],
        risk: CapabilityRisk::Exploit,
        batchable: false,
        max_batch: 1,
        writes: &["findings", "attack_candidates", "evidence_ledger"],
        runner: CapabilityRunnerKind::PentestRunRecipe,
    },
];

pub fn capability_by_id(id: &str) -> Option<&'static StageCapabilitySpec> {
    CAPABILITIES.iter().find(|capability| capability.id == id)
}

pub fn capabilities_for_stage(stage: StageKind) -> Vec<&'static StageCapabilitySpec> {
    CAPABILITIES
        .iter()
        .filter(|capability| capability.stage == stage)
        .collect()
}

pub fn capabilities_for_technique(
    stage: StageKind,
    technique: &str,
) -> Vec<&'static StageCapabilitySpec> {
    CAPABILITIES
        .iter()
        .filter(|capability| {
            capability.stage == stage && capability.techniques.contains(&technique)
        })
        .collect()
}

pub fn suggested_capabilities_for_technique(
    stage: StageKind,
    technique: &str,
) -> Vec<StageCapabilitySuggestion> {
    capabilities_for_technique(stage, technique)
        .into_iter()
        .map(|capability| capability.suggestion(Some(technique)))
        .collect()
}

pub fn suggested_capabilities_for_any_technique(technique: &str) -> Vec<StageCapabilitySuggestion> {
    stage_for_technique(technique)
        .map(|stage| suggested_capabilities_for_technique(stage, technique))
        .unwrap_or_default()
}

pub fn suggested_tools_for_technique(stage: StageKind, technique: &str) -> Vec<String> {
    tools_from_suggestions(&suggested_capabilities_for_technique(stage, technique))
}

pub fn suggested_tools_for_any_technique(technique: &str) -> Vec<String> {
    tools_from_suggestions(&suggested_capabilities_for_any_technique(technique))
}

pub fn stage_for_technique(technique: &str) -> Option<StageKind> {
    if technique.starts_with("GOLISH-INTEL-") {
        Some(StageKind::TargetIntel)
    } else if technique.starts_with("GOLISH-EAS-") {
        Some(StageKind::ExternalAttackSurface)
    } else if technique.starts_with("GOLISH-ENUM-") {
        Some(StageKind::Enumeration)
    } else if technique.starts_with("WSTG-") || technique == "GOLISH-NDAY" {
        Some(StageKind::VulnTriage)
    } else {
        None
    }
}

pub fn tools_from_suggestions(suggestions: &[StageCapabilitySuggestion]) -> Vec<String> {
    let mut tools = Vec::new();
    for suggestion in suggestions {
        for tool in &suggestion.tools {
            if !tools.contains(tool) {
                tools.push(tool.clone());
            }
        }
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{load_embedded_stage_spec, stage_allows, tool_category};
    use serde_json::Value;

    #[test]
    fn eas_and_enumeration_techniques_have_capabilities() {
        for (stage, techniques) in [
            (
                StageKind::ExternalAttackSurface,
                vec![
                    "GOLISH-EAS-LIVENESS",
                    "GOLISH-EAS-PORT",
                    "GOLISH-EAS-SERVICE-FINGERPRINT",
                ],
            ),
            (
                StageKind::Enumeration,
                vec![
                    "GOLISH-ENUM-JS",
                    "GOLISH-ENUM-DIR",
                    "GOLISH-ENUM-PARAM",
                    "GOLISH-ENUM-JSAPI",
                ],
            ),
        ] {
            for technique in techniques {
                assert!(
                    !capabilities_for_technique(stage, technique).is_empty(),
                    "{stage:?} {technique} should map to a capability"
                );
            }
        }
    }

    #[test]
    fn capability_tools_fit_stage_whitelist_or_are_direct_meta_tools() {
        for capability in CAPABILITIES {
            let spec = load_embedded_stage_spec(capability.stage).expect("spec loads");
            for tool in capability.tool_names {
                if tool_category(tool).is_none() {
                    continue;
                }
                assert!(
                    stage_allows(tool, &Value::Null, &spec.allowed_tool_types),
                    "{} tool {} should fit {:?} whitelist {:?}",
                    capability.id,
                    tool,
                    capability.stage,
                    spec.allowed_tool_types
                );
            }
        }
    }

    #[test]
    fn target_intel_capabilities_do_not_expose_scan_cli() {
        let tools = capabilities_for_stage(StageKind::TargetIntel)
            .into_iter()
            .flat_map(|capability| capability.tool_names.iter().copied())
            .collect::<Vec<_>>();
        for forbidden in ["pentest_run", "nmap", "httpx", "naabu", "masscan"] {
            assert!(
                !tools.contains(&forbidden),
                "target_intel should not expose {forbidden}"
            );
        }
    }

    #[test]
    fn enumeration_capabilities_do_not_expose_external_dir_fuzzers() {
        let tools = capabilities_for_stage(StageKind::Enumeration)
            .into_iter()
            .flat_map(|capability| capability.tool_names.iter().copied())
            .collect::<Vec<_>>();
        for forbidden in [
            "ffuf",
            "gobuster",
            "feroxbuster",
            "dirsearch",
            "arjun",
            "pentest_run",
            "katana",
        ] {
            assert!(
                !tools.contains(&forbidden),
                "enumeration should not expose {forbidden}"
            );
        }
    }

    #[test]
    fn enumeration_crawler_suggests_backend_wrapper() {
        let capability = capability_by_id("enum.crawl_same_origin_urls").unwrap();
        assert_eq!(capability.runner, CapabilityRunnerKind::BackendWrapper);
        assert_eq!(capability.tool_names, &["enum_crawl_same_origin_urls"]);
    }

    #[test]
    fn vuln_formulaic_sweep_suggests_backend_wrapper() {
        let capability = capability_by_id("vuln.run_formulaic_sweep").unwrap();
        assert_eq!(capability.runner, CapabilityRunnerKind::BackendWrapper);
        assert_eq!(capability.tool_names, &["vuln_run_formulaic_sweep"]);
        assert!(
            suggested_tools_for_technique(StageKind::VulnTriage, "WSTG-INPV-05")
                .contains(&"vuln_run_formulaic_sweep".to_string())
        );
    }

    #[test]
    fn eas_service_fingerprint_suggests_backend_wrapper() {
        assert_eq!(
            suggested_tools_for_technique(
                StageKind::ExternalAttackSurface,
                "GOLISH-EAS-SERVICE-FINGERPRINT",
            ),
            vec!["eas_fingerprint_services".to_string()]
        );
    }
}
