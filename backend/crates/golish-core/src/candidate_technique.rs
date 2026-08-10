//! Versioned, server-owned Candidate technique methodology registry.
//!
//! The registry describes when a hypothesis may be proposed and what a later
//! Verification campaign must preserve. It is methodology authority only:
//! target facts and Finding truth still come from Tool Truth/evidence and typed
//! oracles respectively.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CANDIDATE_TECHNIQUE_REGISTRY_SCHEMA_V1: &str = "candidate_technique_method_registry.v1";
pub const CANDIDATE_TECHNIQUE_CARD_SCHEMA_V1: &str = "candidate_technique_method_card.v1";

pub const REQUIRED_EXPERIMENT_PHASES_V1: &[&str] = &[
    "baseline",
    "attack",
    "negative_control",
    "reproduction",
    "impact_proof",
    "cleanup",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateTechniqueMethodCardV1 {
    pub schema: String,
    pub technique_id: String,
    pub technique_version: u32,
    pub contract_digest: String,
    pub title: String,
    pub attack_class_id: String,
    pub predicate_schema: String,
    pub predicate_version: u32,
    pub source_framework_refs: Vec<String>,
    pub cwe_ids: Vec<String>,
    pub applicability_signal_ids: Vec<String>,
    pub prerequisite_ids: Vec<String>,
    pub experiment_phases: Vec<String>,
    pub oracle_profile_id: String,
    pub oracle_profile_version: u32,
    pub retest_trigger_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateTechniqueRegistrySnapshotV1 {
    pub schema: String,
    pub registry_version: u32,
    pub registry_manifest_hash: String,
    pub cards: Vec<CandidateTechniqueMethodCardV1>,
}

#[derive(Debug, Clone, Copy)]
struct CardDefinition {
    technique_id: &'static str,
    title: &'static str,
    attack_class_id: &'static str,
    predicate_schema: &'static str,
    source_framework_refs: &'static [&'static str],
    cwe_ids: &'static [&'static str],
    applicability_signal_ids: &'static [&'static str],
    prerequisite_ids: &'static [&'static str],
    oracle_profile_id: &'static str,
    retest_trigger_ids: &'static [&'static str],
}

const STANDARD_RETEST_TRIGGERS: &[&str] = &[
    "application_context_changed",
    "identity_context_changed",
    "new_tool_truth_fact",
    "target_state_epoch_changed",
];

const CARD_DEFINITIONS_V1: &[CardDefinition] = &[
    CardDefinition {
        technique_id: "GOLISH-METHOD-SERVICE-EXPOSURE",
        title: "Network service exposure",
        attack_class_id: "configuration",
        predicate_schema: "network_service_exposure",
        source_framework_refs: &["OWASP-WSTG-v4.2-INFO"],
        cwe_ids: &["CWE-200"],
        applicability_signal_ids: &["reachable_service", "service_fingerprint"],
        prerequisite_ids: &["reachable_service_fact", "service_identity"],
        oracle_profile_id: "service_exposure_differential.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-AUTH-BYPASS",
        title: "Authentication control bypass",
        attack_class_id: "authentication",
        predicate_schema: "authentication_control_bypass",
        source_framework_refs: &["OWASP-WSTG-v4.2-ATHN"],
        cwe_ids: &["CWE-287"],
        applicability_signal_ids: &["authentication_boundary", "protected_function"],
        prerequisite_ids: &[
            "protected_baseline",
            "unauthenticated_or_alternate_identity_context",
        ],
        oracle_profile_id: "authentication_differential.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-IDOR",
        title: "Object authorization differential",
        attack_class_id: "authorization",
        predicate_schema: "object_authorization_bypass",
        source_framework_refs: &["OWASP-WSTG-v4.2-ATHZ-04"],
        cwe_ids: &["CWE-639", "CWE-862"],
        applicability_signal_ids: &["authenticated_object_reference", "object_crud_function"],
        prerequisite_ids: &[
            "principal_a_context",
            "principal_b_context",
            "victim_object_ownership",
            "cross_principal_object_reference",
        ],
        oracle_profile_id: "idor_owner_attacker_control.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-BUSINESS-LOGIC",
        title: "Business invariant violation",
        attack_class_id: "business_logic",
        predicate_schema: "business_invariant_violation",
        source_framework_refs: &["OWASP-WSTG-v4.2-BUSL"],
        cwe_ids: &["CWE-840"],
        applicability_signal_ids: &["multi_step_workflow", "business_state_transition"],
        prerequisite_ids: &["business_invariant", "authoritative_state_reader"],
        oracle_profile_id: "business_invariant_state.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-RACE-CONDITION",
        title: "Race condition and TOCTOU",
        attack_class_id: "business_logic",
        predicate_schema: "race_condition_invariant_violation",
        source_framework_refs: &["CWE-362", "CWE-367"],
        cwe_ids: &["CWE-362", "CWE-367"],
        applicability_signal_ids: &["one_time_or_limited_operation", "state_changing_operation"],
        prerequisite_ids: &[
            "serial_baseline",
            "business_invariant",
            "bounded_concurrency_capability",
            "authoritative_final_state_reader",
        ],
        oracle_profile_id: "race_final_business_state.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-SECURITY-CONFIG",
        title: "Security configuration weakness",
        attack_class_id: "configuration",
        predicate_schema: "security_configuration_weakness",
        source_framework_refs: &["OWASP-WSTG-v4.2-CONF"],
        cwe_ids: &["CWE-16"],
        applicability_signal_ids: &["configuration_surface", "security_header_or_protocol_fact"],
        prerequisite_ids: &["current_configuration_fact", "expected_security_invariant"],
        oracle_profile_id: "configuration_baseline.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-DATA-EXPOSURE",
        title: "Sensitive information exposure",
        attack_class_id: "data_exposure",
        predicate_schema: "sensitive_information_exposure",
        source_framework_refs: &["OWASP-WSTG-v4.2-INFO"],
        cwe_ids: &["CWE-200"],
        applicability_signal_ids: &["sensitive_field", "unexpected_readable_resource"],
        prerequisite_ids: &["response_or_artifact_fact", "sensitivity_context"],
        oracle_profile_id: "sensitive_data_control_compare.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-SQLI",
        title: "SQL injection",
        attack_class_id: "injection",
        predicate_schema: "sql_interpreter_injection",
        source_framework_refs: &["OWASP-WSTG-v4.2-INPV-05"],
        cwe_ids: &["CWE-89"],
        applicability_signal_ids: &["attacker_controlled_input", "database_interpreter_signal"],
        prerequisite_ids: &[
            "controllable_input_binding",
            "database_sink_or_behavior_signal",
        ],
        oracle_profile_id: "sql_injection_differential.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-COMMAND-INJECTION",
        title: "OS command injection",
        attack_class_id: "injection",
        predicate_schema: "command_interpreter_injection",
        source_framework_refs: &["OWASP-WSTG-v4.2-INPV-12"],
        cwe_ids: &["CWE-78"],
        applicability_signal_ids: &["attacker_controlled_input", "command_interpreter_signal"],
        prerequisite_ids: &[
            "controllable_input_binding",
            "command_sink_or_behavior_signal",
        ],
        oracle_profile_id: "command_injection_differential.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-SSRF",
        title: "Server-side request forgery",
        attack_class_id: "injection",
        predicate_schema: "server_side_request_forgery",
        source_framework_refs: &["OWASP-WSTG-v4.2-INPV-19"],
        cwe_ids: &["CWE-918"],
        applicability_signal_ids: &["attacker_controlled_url_value", "server_side_fetch_signal"],
        prerequisite_ids: &[
            "controllable_url_binding",
            "server_side_fetch_sink",
            "unique_correlation_or_response_channel",
        ],
        oracle_profile_id: "ssrf_nonce_or_content.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-SSTI",
        title: "Server-side template injection",
        attack_class_id: "injection",
        predicate_schema: "server_side_template_injection",
        source_framework_refs: &["OWASP-WSTG-v4.2-INPV"],
        cwe_ids: &["CWE-1336"],
        applicability_signal_ids: &[
            "template_rendering_context",
            "attacker_controlled_template_input",
        ],
        prerequisite_ids: &["controllable_template_binding", "server_side_template_sink"],
        oracle_profile_id: "template_evaluation_differential.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-XXE",
        title: "XML external entity processing",
        attack_class_id: "injection",
        predicate_schema: "xml_external_entity_processing",
        source_framework_refs: &["OWASP-WSTG-v4.2-INPV-07"],
        cwe_ids: &["CWE-611"],
        applicability_signal_ids: &["xml_input_surface", "xml_parser_signal"],
        prerequisite_ids: &["controllable_xml_document", "server_side_xml_parser"],
        oracle_profile_id: "xxe_nonce_or_content.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-REQUEST-SMUGGLING",
        title: "HTTP request smuggling",
        attack_class_id: "configuration",
        predicate_schema: "http_request_boundary_desynchronization",
        source_framework_refs: &["OWASP-WSTG-v4.2-INPV-15"],
        cwe_ids: &["CWE-444"],
        applicability_signal_ids: &["multi_hop_http_chain", "parser_or_protocol_boundary"],
        prerequisite_ids: &[
            "front_end_back_end_chain",
            "parser_boundary_signal",
            "isolated_connection_capability",
            "desynchronization_observer",
        ],
        oracle_profile_id: "http_desync_then_impact.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-CORS",
        title: "Cross-origin authorization policy",
        attack_class_id: "authorization",
        predicate_schema: "cross_origin_authorization_weakness",
        source_framework_refs: &["OWASP-WSTG-v4.2-CLNT-07"],
        cwe_ids: &["CWE-942"],
        applicability_signal_ids: &["cors_response_policy", "credentialed_cross_origin_surface"],
        prerequisite_ids: &["trusted_origin_baseline", "untrusted_origin_control"],
        oracle_profile_id: "cors_origin_control_compare.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-JWT",
        title: "JSON Web Token validation",
        attack_class_id: "authentication",
        predicate_schema: "jwt_validation_weakness",
        source_framework_refs: &["OWASP-WSTG-v4.2-SESS"],
        cwe_ids: &["CWE-345"],
        applicability_signal_ids: &["jwt_token_surface", "token_protected_function"],
        prerequisite_ids: &["valid_token_baseline", "token_validation_boundary"],
        oracle_profile_id: "jwt_mutation_control_compare.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-GRAPHQL",
        title: "GraphQL authorization and query controls",
        attack_class_id: "authorization",
        predicate_schema: "graphql_security_control_weakness",
        source_framework_refs: &["OWASP-WSTG-v4.2-APIT"],
        cwe_ids: &["CWE-285", "CWE-770"],
        applicability_signal_ids: &["graphql_endpoint", "graphql_operation_or_schema_signal"],
        prerequisite_ids: &[
            "graphql_operation_binding",
            "identity_or_complexity_control",
        ],
        oracle_profile_id: "graphql_control_compare.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-HOST-HEADER",
        title: "Host header trust boundary",
        attack_class_id: "configuration",
        predicate_schema: "host_header_trust_weakness",
        source_framework_refs: &["OWASP-WSTG-v4.2-INPV-17"],
        cwe_ids: &["CWE-346"],
        applicability_signal_ids: &["host_derived_link_or_routing", "proxy_host_boundary"],
        prerequisite_ids: &["canonical_host_baseline", "host_mutation_observer"],
        oracle_profile_id: "host_header_differential.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-OPEN-REDIRECT",
        title: "Unvalidated redirect",
        attack_class_id: "injection",
        predicate_schema: "unvalidated_redirect",
        source_framework_refs: &["OWASP-WSTG-v4.2-CLNT-04"],
        cwe_ids: &["CWE-601"],
        applicability_signal_ids: &["attacker_controlled_redirect_target", "redirect_sink"],
        prerequisite_ids: &["controllable_redirect_binding", "external_origin_control"],
        oracle_profile_id: "redirect_origin_compare.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-PROTOTYPE-POLLUTION",
        title: "Prototype pollution",
        attack_class_id: "injection",
        predicate_schema: "prototype_pollution",
        source_framework_refs: &["CWE-1321"],
        cwe_ids: &["CWE-1321"],
        applicability_signal_ids: &[
            "javascript_object_merge",
            "attacker_controlled_property_path",
        ],
        prerequisite_ids: &["controllable_property_binding", "prototype_sensitive_sink"],
        oracle_profile_id: "prototype_state_control_compare.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-RATE-LIMIT",
        title: "Rate-limit and quota bypass",
        attack_class_id: "business_logic",
        predicate_schema: "rate_limit_or_quota_bypass",
        source_framework_refs: &["OWASP-WSTG-v4.2-BUSL"],
        cwe_ids: &["CWE-770"],
        applicability_signal_ids: &["rate_limited_function", "quota_identity_key"],
        prerequisite_ids: &[
            "single_identity_limit_baseline",
            "bounded_alternate_key_strategy",
        ],
        oracle_profile_id: "quota_state_compare.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-SUBDOMAIN-TAKEOVER",
        title: "Dangling delegated service binding",
        attack_class_id: "configuration",
        predicate_schema: "dangling_service_binding",
        source_framework_refs: &["OWASP-WSTG-v4.2-CONF"],
        cwe_ids: &["CWE-16"],
        applicability_signal_ids: &[
            "dangling_dns_or_service_reference",
            "unclaimed_provider_resource",
        ],
        prerequisite_ids: &["current_dns_or_service_binding", "provider_unclaimed_state"],
        oracle_profile_id: "dangling_binding_control.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-WEBSOCKET",
        title: "WebSocket authorization and origin controls",
        attack_class_id: "authorization",
        predicate_schema: "websocket_security_control_weakness",
        source_framework_refs: &["OWASP-WSTG-v4.2-CLNT"],
        cwe_ids: &["CWE-285"],
        applicability_signal_ids: &[
            "websocket_endpoint",
            "websocket_identity_or_origin_boundary",
        ],
        prerequisite_ids: &[
            "valid_session_baseline",
            "alternate_identity_or_origin_control",
        ],
        oracle_profile_id: "websocket_control_compare.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-CACHE-POISONING",
        title: "Web cache key confusion",
        attack_class_id: "configuration",
        predicate_schema: "web_cache_key_confusion",
        source_framework_refs: &["CWE-444"],
        cwe_ids: &["CWE-444"],
        applicability_signal_ids: &["shared_cache_surface", "unkeyed_input_signal"],
        prerequisite_ids: &[
            "uncached_baseline",
            "cache_hit_observer",
            "isolated_cache_key",
        ],
        oracle_profile_id: "cache_key_control_compare.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-AVAILABILITY",
        title: "Bounded resource exhaustion",
        attack_class_id: "availability",
        predicate_schema: "resource_exhaustion_weakness",
        source_framework_refs: &["CWE-400"],
        cwe_ids: &["CWE-400"],
        applicability_signal_ids: &["amplifiable_resource_operation", "resource_limit_signal"],
        prerequisite_ids: &["safe_load_baseline", "bounded_resource_observer"],
        oracle_profile_id: "resource_use_bounded_compare.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
    CardDefinition {
        technique_id: "GOLISH-METHOD-VULNERABLE-COMPONENT",
        title: "Known vulnerable component applicability",
        attack_class_id: "supply_chain",
        predicate_schema: "known_vulnerable_component_applicability",
        source_framework_refs: &["CWE-1104"],
        cwe_ids: &["CWE-1104"],
        applicability_signal_ids: &["product_identity", "versioned_advisory_match"],
        prerequisite_ids: &[
            "current_product_version",
            "signed_advisory_match",
            "target_reachability",
        ],
        oracle_profile_id: "version_and_behavior_confirmation.v1",
        retest_trigger_ids: STANDARD_RETEST_TRIGGERS,
    },
];

struct DomainHashWriter(Sha256);

impl DomainHashWriter {
    fn new(domain: &str) -> Self {
        let mut digest = Sha256::new();
        digest.update(domain.as_bytes());
        digest.update([0]);
        Self(digest)
    }

    fn field(&mut self, value: &[u8]) {
        self.0.update((value.len() as u64).to_be_bytes());
        self.0.update(value);
    }

    fn text(&mut self, value: &str) {
        self.field(value.as_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.field(&value.to_be_bytes());
    }

    fn texts(&mut self, values: &[String]) {
        self.u32(u32::try_from(values.len()).unwrap_or(u32::MAX));
        for value in values {
            self.text(value);
        }
    }

    fn finish(self) -> String {
        let mut output = String::from("sha256:");
        for byte in self.0.finalize() {
            use std::fmt::Write as _;
            write!(output, "{byte:02x}").expect("writing to String cannot fail");
        }
        output
    }
}

fn text_vec(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn card_digest(card: &CandidateTechniqueMethodCardV1) -> String {
    let mut hash = DomainHashWriter::new(CANDIDATE_TECHNIQUE_CARD_SCHEMA_V1);
    hash.text(&card.technique_id);
    hash.u32(card.technique_version);
    hash.text(&card.title);
    hash.text(&card.attack_class_id);
    hash.text(&card.predicate_schema);
    hash.u32(card.predicate_version);
    hash.texts(&card.source_framework_refs);
    hash.texts(&card.cwe_ids);
    hash.texts(&card.applicability_signal_ids);
    hash.texts(&card.prerequisite_ids);
    hash.texts(&card.experiment_phases);
    hash.text(&card.oracle_profile_id);
    hash.u32(card.oracle_profile_version);
    hash.texts(&card.retest_trigger_ids);
    hash.finish()
}

fn snapshot(definition: CardDefinition) -> CandidateTechniqueMethodCardV1 {
    let mut card = CandidateTechniqueMethodCardV1 {
        schema: CANDIDATE_TECHNIQUE_CARD_SCHEMA_V1.to_owned(),
        technique_id: definition.technique_id.to_owned(),
        technique_version: 1,
        contract_digest: String::new(),
        title: definition.title.to_owned(),
        attack_class_id: definition.attack_class_id.to_owned(),
        predicate_schema: definition.predicate_schema.to_owned(),
        predicate_version: 1,
        source_framework_refs: text_vec(definition.source_framework_refs),
        cwe_ids: text_vec(definition.cwe_ids),
        applicability_signal_ids: text_vec(definition.applicability_signal_ids),
        prerequisite_ids: text_vec(definition.prerequisite_ids),
        experiment_phases: text_vec(REQUIRED_EXPERIMENT_PHASES_V1),
        oracle_profile_id: definition.oracle_profile_id.to_owned(),
        oracle_profile_version: 1,
        retest_trigger_ids: text_vec(definition.retest_trigger_ids),
    };
    card.contract_digest = card_digest(&card);
    card
}

pub fn candidate_technique_method_cards_v1() -> Vec<CandidateTechniqueMethodCardV1> {
    CARD_DEFINITIONS_V1.iter().copied().map(snapshot).collect()
}

pub fn candidate_technique_method_card_v1(
    technique_id: &str,
) -> Option<CandidateTechniqueMethodCardV1> {
    CARD_DEFINITIONS_V1
        .iter()
        .copied()
        .find(|definition| definition.technique_id == technique_id)
        .map(snapshot)
}

pub fn candidate_technique_method_card_for_predicate_v1(
    predicate_schema: &str,
    predicate_version: u32,
) -> Option<CandidateTechniqueMethodCardV1> {
    (predicate_version == 1)
        .then(|| {
            CARD_DEFINITIONS_V1
                .iter()
                .copied()
                .find(|definition| definition.predicate_schema == predicate_schema)
                .map(snapshot)
        })
        .flatten()
}

pub fn candidate_technique_method_cards_for_attack_class_v1(
    attack_class_id: &str,
) -> Vec<CandidateTechniqueMethodCardV1> {
    CARD_DEFINITIONS_V1
        .iter()
        .copied()
        .filter(|definition| definition.attack_class_id == attack_class_id)
        .map(snapshot)
        .collect()
}

pub fn candidate_technique_method_card_set_hash_v1(
    cards: &[CandidateTechniqueMethodCardV1],
) -> String {
    let mut identities = cards
        .iter()
        .map(|card| {
            (
                card.technique_id.as_str(),
                card.technique_version,
                card.contract_digest.as_str(),
            )
        })
        .collect::<Vec<_>>();
    identities.sort_unstable();
    let mut hash = DomainHashWriter::new("candidate_technique_method_card_set.v1");
    hash.u32(u32::try_from(identities.len()).unwrap_or(u32::MAX));
    for (technique_id, technique_version, contract_digest) in identities {
        hash.text(technique_id);
        hash.u32(technique_version);
        hash.text(contract_digest);
    }
    hash.finish()
}

pub fn candidate_technique_registry_snapshot_v1() -> CandidateTechniqueRegistrySnapshotV1 {
    let cards = candidate_technique_method_cards_v1();
    CandidateTechniqueRegistrySnapshotV1 {
        schema: CANDIDATE_TECHNIQUE_REGISTRY_SCHEMA_V1.to_owned(),
        registry_version: 1,
        registry_manifest_hash: candidate_technique_method_card_set_hash_v1(&cards),
        cards,
    }
}

pub fn validate_candidate_technique_card_v1(card: &CandidateTechniqueMethodCardV1) -> bool {
    card.schema == CANDIDATE_TECHNIQUE_CARD_SCHEMA_V1
        && card.technique_version == 1
        && card.predicate_version == 1
        && card.oracle_profile_version == 1
        && !card.technique_id.trim().is_empty()
        && !card.attack_class_id.trim().is_empty()
        && !card.predicate_schema.trim().is_empty()
        && !card.source_framework_refs.is_empty()
        && !card.cwe_ids.is_empty()
        && !card.applicability_signal_ids.is_empty()
        && !card.prerequisite_ids.is_empty()
        && card.experiment_phases
            == REQUIRED_EXPERIMENT_PHASES_V1
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        && !card.oracle_profile_id.trim().is_empty()
        && unique_nonempty(&card.source_framework_refs)
        && unique_nonempty(&card.cwe_ids)
        && unique_nonempty(&card.applicability_signal_ids)
        && unique_nonempty(&card.prerequisite_ids)
        && unique_nonempty(&card.retest_trigger_ids)
        && card.contract_digest == card_digest(card)
}

fn unique_nonempty(values: &[String]) -> bool {
    values.iter().all(|value| !value.trim().is_empty())
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_technique_registry_is_unique_complete_and_self_authenticating() {
        let registry = candidate_technique_registry_snapshot_v1();
        assert_eq!(registry.registry_version, 1);
        assert!(registry.cards.len() >= 20);
        assert_eq!(
            registry.registry_manifest_hash,
            candidate_technique_method_card_set_hash_v1(&registry.cards)
        );
        assert!(registry
            .cards
            .iter()
            .all(validate_candidate_technique_card_v1));
        assert_eq!(
            registry
                .cards
                .iter()
                .map(|card| (&card.technique_id, card.technique_version))
                .collect::<BTreeSet<_>>()
                .len(),
            registry.cards.len()
        );
        assert_eq!(
            registry
                .cards
                .iter()
                .map(|card| (&card.predicate_schema, card.predicate_version))
                .collect::<BTreeSet<_>>()
                .len(),
            registry.cards.len()
        );
    }

    #[test]
    fn candidate_technique_card_tamper_changes_digest_and_is_rejected() {
        let mut card =
            candidate_technique_method_card_v1("GOLISH-METHOD-IDOR").expect("IDOR method card");
        assert!(validate_candidate_technique_card_v1(&card));
        card.prerequisite_ids.pop();
        assert!(!validate_candidate_technique_card_v1(&card));
    }

    #[test]
    fn candidate_technique_idor_requires_two_identities_and_ownership() {
        let card =
            candidate_technique_method_card_v1("GOLISH-METHOD-IDOR").expect("IDOR method card");
        assert_eq!(card.predicate_schema, "object_authorization_bypass");
        assert_eq!(
            card.prerequisite_ids,
            [
                "principal_a_context",
                "principal_b_context",
                "victim_object_ownership",
                "cross_principal_object_reference",
            ]
        );
        assert_eq!(card.oracle_profile_id, "idor_owner_attacker_control.v1");
    }

    #[test]
    fn candidate_technique_unknown_predicate_has_no_method_authority() {
        assert!(candidate_technique_method_card_for_predicate_v1("invented_vuln", 1).is_none());
        assert!(
            candidate_technique_method_card_for_predicate_v1("object_authorization_bypass", 2)
                .is_none()
        );
    }
}
