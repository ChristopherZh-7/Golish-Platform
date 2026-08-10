//! Host-owned Investigation operator-tool catalog and fail-closed admission.
//!
//! The catalog is inventory policy, not an execution grant. Cognitive workers
//! never receive these tools. A host Operator may select one only after exact
//! target/scope, adapter, credential, external-service and JIT authority has
//! been re-derived for the current action.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const INVESTIGATION_OPERATOR_TOOL_CONTRACT_V1: &str = "investigation_operator_tools_v1";
pub const INVESTIGATION_OPERATOR_TOOL_CATALOG_RESOURCE_V1: &str =
    "resources/harness/stages/investigation/tool_catalog.json";
pub const INVESTIGATION_TOOL_CONFIG_IDS: [&str; 10] = [
    "arjun",
    "kiterunner",
    "schemathesis",
    "jwt-tool",
    "graphql-cop",
    "testssl-sh",
    "ssh-audit",
    "enum4linux-ng",
    "trivy",
    "interactsh-client",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationToolExecutionClassV1 {
    NetworkObserve,
    ActiveBounded,
    LocalThenActive,
    StatefulFuzz,
    LocalOrRegistryRead,
    ExternalOast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationToolAvailabilityV1 {
    AutomaticExactTarget,
    JitOnly,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationToolContractStatusV1 {
    Ready,
    ContractPending,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationToolTerminalTruthV1 {
    TypedAdapterRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationTypedAdapterRefV1 {
    pub contract_id: String,
    pub contract_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationOperatorToolProfileV1 {
    pub tool_config_id: String,
    pub capability: String,
    pub execution_class: InvestigationToolExecutionClassV1,
    pub default_availability: InvestigationToolAvailabilityV1,
    pub contract_status: InvestigationToolContractStatusV1,
    pub target_kinds: Vec<String>,
    pub credential_mode: String,
    pub external_service: bool,
    pub terminal_truth: InvestigationToolTerminalTruthV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typed_adapter: Option<InvestigationTypedAdapterRefV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationOperatorToolCatalogV1 {
    pub contract_version: String,
    pub tools: Vec<InvestigationOperatorToolProfileV1>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InvestigationToolCatalogError {
    #[error("invalid Investigation operator catalog JSON: {0}")]
    Json(String),
    #[error("invalid Investigation operator catalog: {0}")]
    Contract(String),
}

impl InvestigationOperatorToolCatalogV1 {
    pub fn parse_and_validate(raw: &str) -> Result<Self, InvestigationToolCatalogError> {
        let catalog: Self = serde_json::from_str(raw)
            .map_err(|error| InvestigationToolCatalogError::Json(error.to_string()))?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<(), InvestigationToolCatalogError> {
        if self.contract_version != INVESTIGATION_OPERATOR_TOOL_CONTRACT_V1 {
            return Err(InvestigationToolCatalogError::Contract(
                "unknown contract_version".to_string(),
            ));
        }
        let mut ids = BTreeSet::new();
        let mut capabilities = BTreeSet::new();
        for profile in &self.tools {
            if profile.tool_config_id.trim().is_empty()
                || profile.capability.trim().is_empty()
                || profile.credential_mode.trim().is_empty()
                || profile.target_kinds.is_empty()
                || profile
                    .target_kinds
                    .iter()
                    .any(|target| target.trim().is_empty())
            {
                return Err(InvestigationToolCatalogError::Contract(
                    "tool id, capability, credential mode and target kinds must be non-empty"
                        .to_string(),
                ));
            }
            if !ids.insert(profile.tool_config_id.as_str()) {
                return Err(InvestigationToolCatalogError::Contract(format!(
                    "duplicate tool_config_id {}",
                    profile.tool_config_id
                )));
            }
            if !capabilities.insert(profile.capability.as_str()) {
                return Err(InvestigationToolCatalogError::Contract(format!(
                    "duplicate capability {}",
                    profile.capability
                )));
            }
            let target_set = profile
                .target_kinds
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            if target_set.len() != profile.target_kinds.len() {
                return Err(InvestigationToolCatalogError::Contract(format!(
                    "duplicate target kind for {}",
                    profile.tool_config_id
                )));
            }
            if profile.execution_class == InvestigationToolExecutionClassV1::ExternalOast
                && !profile.external_service
            {
                return Err(InvestigationToolCatalogError::Contract(
                    "external_oast must declare external_service=true".to_string(),
                ));
            }
            if matches!(
                profile.execution_class,
                InvestigationToolExecutionClassV1::StatefulFuzz
                    | InvestigationToolExecutionClassV1::ExternalOast
            ) && profile.default_availability != InvestigationToolAvailabilityV1::Disabled
            {
                return Err(InvestigationToolCatalogError::Contract(
                    "stateful_fuzz and external_oast must default to disabled".to_string(),
                ));
            }
            if profile.default_availability == InvestigationToolAvailabilityV1::AutomaticExactTarget
                && (profile.execution_class != InvestigationToolExecutionClassV1::NetworkObserve
                    || profile.external_service)
            {
                return Err(InvestigationToolCatalogError::Contract(
                    "only non-external network_observe tools may default to automatic_exact_target"
                        .to_string(),
                ));
            }
            if profile.contract_status == InvestigationToolContractStatusV1::Ready
                && profile.typed_adapter.is_none()
            {
                return Err(InvestigationToolCatalogError::Contract(format!(
                    "ready tool {} has no typed adapter",
                    profile.tool_config_id
                )));
            }
            if let Some(adapter) = profile.typed_adapter.as_ref() {
                if adapter.contract_id.trim().is_empty()
                    || !is_lower_hex_sha256(&adapter.contract_digest)
                {
                    return Err(InvestigationToolCatalogError::Contract(format!(
                        "tool {} has an invalid typed adapter identity",
                        profile.tool_config_id
                    )));
                }
            }
        }
        let expected = INVESTIGATION_TOOL_CONFIG_IDS
            .into_iter()
            .collect::<BTreeSet<_>>();
        if ids != expected {
            return Err(InvestigationToolCatalogError::Contract(
                "tool_config_id set does not match the frozen first-batch exact set".to_string(),
            ));
        }
        Ok(())
    }

    pub fn contract_sha256(&self) -> String {
        let canonical = serde_json::to_vec(self)
            .expect("validated Investigation operator catalog must serialize");
        let mut hasher = Sha256::new();
        hasher.update(b"golish.investigation-operator-tool-catalog.v1\0");
        hasher.update(canonical);
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn profile(&self, tool_config_id: &str) -> Option<&InvestigationOperatorToolProfileV1> {
        self.tools
            .iter()
            .find(|profile| profile.tool_config_id == tool_config_id)
    }
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationToolAdmissionRequestV1 {
    pub tool_config_id: String,
    pub actor_is_cognitive: bool,
    pub target_kind: String,
    pub exact_scope_authority: bool,
    pub target_write_guard_passed: bool,
    pub fuel_reserved: bool,
    pub lease_current: bool,
    pub jit_authorized: bool,
    pub exact_credential_grant: bool,
    pub external_service_authorized: bool,
    pub adapter_contract_id: String,
    pub adapter_contract_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedInvestigationOperatorToolV1 {
    pub tool_config_id: String,
    pub capability: String,
    pub execution_class: InvestigationToolExecutionClassV1,
    pub adapter_contract_id: String,
    pub adapter_contract_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("Investigation operator admission rejected: {reason_code}")]
pub struct InvestigationToolAdmissionRejectionV1 {
    pub reason_code: &'static str,
}

pub fn admit_investigation_operator_tool(
    catalog: &InvestigationOperatorToolCatalogV1,
    request: &InvestigationToolAdmissionRequestV1,
) -> Result<AdmittedInvestigationOperatorToolV1, InvestigationToolAdmissionRejectionV1> {
    let reject = |reason_code| InvestigationToolAdmissionRejectionV1 { reason_code };
    if request.actor_is_cognitive {
        return Err(reject("cognitive_actor_has_no_operator_authority"));
    }
    let profile = catalog
        .profile(&request.tool_config_id)
        .ok_or_else(|| reject("tool_not_in_frozen_catalog"))?;
    if profile.contract_status != InvestigationToolContractStatusV1::Ready {
        return Err(reject("tool_contract_not_ready"));
    }
    if !profile
        .target_kinds
        .iter()
        .any(|kind| kind == &request.target_kind)
    {
        return Err(reject("target_kind_not_admitted"));
    }
    if !request.exact_scope_authority
        || !request.target_write_guard_passed
        || !request.fuel_reserved
        || !request.lease_current
    {
        return Err(reject("launch_authority_incomplete"));
    }
    let adapter = profile
        .typed_adapter
        .as_ref()
        .ok_or_else(|| reject("typed_adapter_missing"))?;
    if adapter.contract_id != request.adapter_contract_id
        || adapter.contract_digest != request.adapter_contract_digest
    {
        return Err(reject("typed_adapter_identity_mismatch"));
    }
    if profile.credential_mode != "none" && profile.credential_mode != "none_or_exact_grant" {
        return Err(reject("credential_mode_not_supported"));
    }
    if profile.credential_mode == "none_or_exact_grant" && !request.exact_credential_grant {
        return Err(reject("exact_credential_grant_missing"));
    }
    if profile.external_service && !request.external_service_authorized {
        return Err(reject("external_service_authority_missing"));
    }
    match profile.default_availability {
        InvestigationToolAvailabilityV1::Disabled => {
            return Err(reject("tool_disabled_by_catalog"));
        }
        InvestigationToolAvailabilityV1::JitOnly if !request.jit_authorized => {
            return Err(reject("jit_authorization_missing"));
        }
        InvestigationToolAvailabilityV1::AutomaticExactTarget
            if profile.execution_class != InvestigationToolExecutionClassV1::NetworkObserve
                || profile.external_service
                || profile.credential_mode != "none" =>
        {
            return Err(reject("automatic_policy_not_satisfied"));
        }
        _ => {}
    }
    Ok(AdmittedInvestigationOperatorToolV1 {
        tool_config_id: profile.tool_config_id.clone(),
        capability: profile.capability.clone(),
        execution_class: profile.execution_class,
        adapter_contract_id: adapter.contract_id.clone(),
        adapter_contract_digest: adapter.contract_digest.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_network_catalog() -> InvestigationOperatorToolCatalogV1 {
        let mut catalog = InvestigationOperatorToolCatalogV1::parse_and_validate(
            crate::harness::resources::investigation_tool_catalog_json(),
        )
        .unwrap();
        let profile = catalog
            .tools
            .iter_mut()
            .find(|profile| profile.tool_config_id == "testssl-sh")
            .unwrap();
        profile.contract_status = InvestigationToolContractStatusV1::Ready;
        profile.typed_adapter = Some(InvestigationTypedAdapterRefV1 {
            contract_id: "typed.testssl.v1".to_string(),
            contract_digest: "a".repeat(64),
        });
        catalog.validate().unwrap();
        catalog
    }

    fn request() -> InvestigationToolAdmissionRequestV1 {
        InvestigationToolAdmissionRequestV1 {
            tool_config_id: "testssl-sh".to_string(),
            actor_is_cognitive: false,
            target_kind: "endpoint".to_string(),
            exact_scope_authority: true,
            target_write_guard_passed: true,
            fuel_reserved: true,
            lease_current: true,
            jit_authorized: false,
            exact_credential_grant: true,
            external_service_authorized: false,
            adapter_contract_id: "typed.testssl.v1".to_string(),
            adapter_contract_digest: "a".repeat(64),
        }
    }

    #[test]
    fn cognitive_roles_never_receive_operator_admission() {
        let catalog = ready_network_catalog();
        let mut request = request();
        request.actor_is_cognitive = true;
        assert_eq!(
            admit_investigation_operator_tool(&catalog, &request)
                .unwrap_err()
                .reason_code,
            "cognitive_actor_has_no_operator_authority"
        );
    }

    #[test]
    fn automatic_network_observe_still_requires_every_host_authority() {
        let catalog = ready_network_catalog();
        assert!(admit_investigation_operator_tool(&catalog, &request()).is_ok());
        for mutate in [
            |request: &mut InvestigationToolAdmissionRequestV1| {
                request.exact_scope_authority = false
            },
            |request: &mut InvestigationToolAdmissionRequestV1| {
                request.target_write_guard_passed = false
            },
            |request: &mut InvestigationToolAdmissionRequestV1| request.fuel_reserved = false,
            |request: &mut InvestigationToolAdmissionRequestV1| request.lease_current = false,
        ] {
            let mut denied = request();
            mutate(&mut denied);
            assert!(admit_investigation_operator_tool(&catalog, &denied).is_err());
        }
    }
}
