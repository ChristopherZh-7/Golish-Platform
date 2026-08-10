//! Agent tools that let the harness `target_intel` stage drive the passive
//! asset-intel engine directly (设计 2026-06-06-intel-stage-ai-driven-per-mode
//! §3.3). These wrap [`crate::asset_intel::run_passive_intel`] so the AI — not a
//! GUI button — performs subsidiary discovery (`recon_discover_subsidiaries`,
//! ENScan), provider asset survey (`recon_map_assets`, 0.zone / quake / …) and
//! WHOIS lookup (`recon_lookup_whois`, RDAP).
//!
//! Both take the confirmed engagement `organization_id` (created during scoping,
//! org-first) and are project-scoped so a tool can never touch another project's
//! org (IDOR guard, AGENTS.md I2). The tool result is a JSON summary that the
//! runtime books to the evidence ledger so `target_intel` coverage cells can cite
//! a real evidence id.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use golish_core::Tool;
use golish_pentest_domain::models::{AssetIntelPivot, AssetIntelPivotKind, IntelSearchIntent};
use golish_pentest_domain::{canonical_asset_key, AssetClass};

use crate::asset_intel::ToolsConfigState;
use crate::asset_intel::{
    freeze_company_lookup_result, land_semantic_goal_observations, list_provider_availability,
    lookup_company_matches, run_passive_intel, run_passive_intel_observation_only,
    AssetIntelHydrateConfig, PassiveIntelPhase, SemanticPivotLandingContext,
};
use crate::intel_providers::{
    target_intel_fixed_provider_endpoints, target_intel_provider_endpoints,
    TargetIntelReceiptBegin, TargetIntelReceiptFinalization, TargetIntelReceiptObserver,
    TargetIntelTechniqueObservation,
};

/// Closed model-visible schema for the fixture/dev semantic Goal Loop. Scope,
/// provider selection, DSL, authorization and evidence destinations are all
/// host-owned and therefore absent.
pub fn recon_search_intel_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "organization_id": {
                "type": "string",
                "format": "uuid"
            },
            "pivot": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": [
                            "company_name", "brand", "domain", "hostname", "ip", "cidr",
                            "asn", "certificate", "icp", "email_domain", "github_org",
                            "repository", "app_id"
                        ]
                    },
                    "value": { "type": "string", "minLength": 1, "maxLength": 512 }
                },
                "required": ["kind", "value"]
            },
            "intent": {
                "type": "string",
                "enum": ["discover_related_assets", "verify_attribution", "enrich_known_asset"]
            }
        },
        "required": ["organization_id", "pivot", "intent"]
    })
}

/// JSON schema shared by both passive recon tools (free function so it is unit
/// testable without a live `PgPool`).
fn passive_intel_parameters(subject_hint: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "organization_id": {
                "type": "string",
                "description": format!("Organization UUID (the confirmed engagement subject {subject_hint}). Create/select it first via manage_organizations.")
            }
        },
        "required": ["organization_id"]
    })
}

/// JSON schema for `recon_map_assets`. Same `organization_id` as the shared
/// passive schema, plus the optional b1 `domain` repair knob (design 2026-06-24):
/// when set, the survey runs domain-keyed — only providers/queries that reference
/// `{{domain}}` fire (e.g. FOFA `domain="x"`) — for a specific apex. The normal
/// org/company survey auto-expands bounded owned apexes after discovery.
fn map_assets_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "organization_id": {
                "type": "string",
                "description": "Organization UUID (the confirmed engagement subject to survey assets for). Create/select it first via manage_organizations."
            },
            "domain": {
                "type": "string",
                "description": "Optional apex domain (e.g. \"example.com\") for a targeted repair/manual supplement. When provided, runs only DOMAIN-keyed provider templates (FOFA domain=\"…\", 0.zone root_domain==…, etc.) for that apex. Omit for the normal org/company survey; the normal call already auto-expands bounded owned apexes it discovers."
            }
        },
        "required": ["organization_id"]
    })
}

/// JSON schema for `recon_discover_subsidiaries`. Adds the scope knobs the
/// scoping agent must ASK the human for (ownership threshold / branches) — see
/// scoping.methodology.md. Absent fields fall back to provider-config defaults.
fn subsidiary_intel_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "organization_id": {
                "type": "string",
                "description": "Organization UUID (the confirmed engagement subject to discover subsidiaries for). Create/select it first via manage_organizations."
            },
            "min_ownership_percent": {
                "type": "string",
                "description": "Ownership threshold (percent, no % sign), e.g. \"51\" or \"100\". A discovered subsidiary auto-promotes into an in-scope child org only when its ownership >= this value. ASK THE HUMAN for this during scoping and pass their answer. Omit to use the provider default (51)."
            },
            "include_branches": {
                "type": "boolean",
                "description": "Also collect branch offices (分公司). Default false. Ask the human whether branches are in scope."
            }
        },
        "required": ["organization_id"]
    })
}

fn organization_visible_in_workspace(organization_project_path: &str, workspace: &Path) -> bool {
    organization_project_path.is_empty()
        || organization_project_path == workspace.to_string_lossy().as_ref()
}

fn resolve_bound_organization_id(requested: Uuid, bound: Option<Uuid>) -> Result<Uuid, String> {
    if let Some(bound) = bound {
        if requested != bound {
            return Err(
                "organization_id does not match the current stage-run organization".to_string(),
            );
        }
        return Ok(bound);
    }
    Ok(requested)
}

fn semantic_value_matches(kind: AssetIntelPivotKind, left: &str, right: &str) -> bool {
    let normalize = |value: &str| match kind {
        AssetIntelPivotKind::Domain
        | AssetIntelPivotKind::Hostname
        | AssetIntelPivotKind::EmailDomain => {
            value.trim().trim_end_matches('.').to_ascii_lowercase()
        }
        _ => value.trim().to_ascii_lowercase(),
    };
    normalize(left) == normalize(right)
}

fn json_contains_semantic_value(value: &Value, kind: AssetIntelPivotKind, expected: &str) -> bool {
    match value {
        Value::String(value) => semantic_value_matches(kind, value, expected),
        Value::Array(values) => values
            .iter()
            .any(|value| json_contains_semantic_value(value, kind, expected)),
        Value::Object(values) => values
            .values()
            .any(|value| json_contains_semantic_value(value, kind, expected)),
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SemanticPivotAuthorization {
    FrozenCompanyIdentity,
    FormalTarget,
    PriorObservationCandidateOnly,
}

impl SemanticPivotAuthorization {
    fn authorization_basis(self) -> &'static str {
        match self {
            Self::FrozenCompanyIdentity => "frozen_company_identity",
            Self::FormalTarget => "formal_target",
            Self::PriorObservationCandidateOnly => "prior_observation_candidate_only",
        }
    }

    fn candidate_only(self) -> bool {
        matches!(self, Self::PriorObservationCandidateOnly)
    }

    fn grants_scope_authority(self) -> bool {
        matches!(self, Self::FormalTarget)
    }
}

fn formal_target_matches_pivot(
    target_type: &str,
    target_value: &str,
    pivot: &AssetIntelPivot,
) -> bool {
    let expected_type = match pivot.kind {
        AssetIntelPivotKind::Domain | AssetIntelPivotKind::Hostname => "domain",
        AssetIntelPivotKind::Ip => "ip",
        AssetIntelPivotKind::Cidr => "cidr",
        _ => return false,
    };
    target_type.eq_ignore_ascii_case(expected_type)
        && semantic_value_matches(pivot.kind, target_value, &pivot.value)
}

fn classify_semantic_pivot_authorization<O>(
    _kind: AssetIntelPivotKind,
    intent: IntelSearchIntent,
    frozen_identity_match: bool,
    prior_observation_match: O,
    formal_target_match: bool,
) -> Result<SemanticPivotAuthorization, &'static str>
where
    O: Into<Option<bool>>,
{
    if formal_target_match {
        return Ok(SemanticPivotAuthorization::FormalTarget);
    }
    if frozen_identity_match {
        return Ok(SemanticPivotAuthorization::FrozenCompanyIdentity);
    }
    if prior_observation_match.into() == Some(true) && intent != IntelSearchIntent::EnrichKnownAsset
    {
        return Ok(SemanticPivotAuthorization::PriorObservationCandidateOnly);
    }
    Err("INTEL_PIVOT_NOT_IN_AUTHORIZED_FRONTIER")
}

/// A semantic pivot can only continue from frozen Company Identity, formal
/// scope, or an exact prior Observation in this operation. This lets the AI
/// change search direction without turning a guessed value into scope.
async fn semantic_pivot_authorized(
    pool: &PgPool,
    organization_id: Uuid,
    pivot: &AssetIntelPivot,
    intent: IntelSearchIntent,
) -> Result<SemanticPivotAuthorization, &'static str> {
    let context = golish_core::current_agent_tool_context()
        .ok_or("TARGET_INTEL_TRUSTED_TOOL_CONTEXT_MISSING")?;
    let operation_id = context
        .operation_id
        .ok_or("TARGET_INTEL_OPERATION_CONTEXT_MISSING")?;
    let identity = sqlx::query_as::<_, (String, Value, Value, Value, Value, Value, Value)>(
        r#"SELECT identity.canonical_legal_name,identity.aliases,identity.brands,
                  identity.registration_identifiers,identity.disambiguation_fields,
                  identity.scope_policy,identity.identity_payload
             FROM target_intel_goal_company_identity_bindings binding
             JOIN scoping_company_identity_receipts identity
               ON identity.id=binding.company_identity_receipt_id
              AND identity.operation_id=binding.operation_id
              AND identity.organization_id=binding.organization_id
            WHERE binding.operation_id=$1 AND binding.organization_id=$2"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| "TARGET_INTEL_COMPANY_IDENTITY_READ_FAILED")?
    .ok_or("TARGET_INTEL_COMPANY_IDENTITY_MISSING")?;
    let identity_match = semantic_value_matches(pivot.kind, &identity.0, &pivot.value)
        || [
            &identity.1,
            &identity.2,
            &identity.3,
            &identity.4,
            &identity.5,
            &identity.6,
        ]
        .into_iter()
        .any(|value| json_contains_semantic_value(value, pivot.kind, &pivot.value));
    if matches!(
        pivot.kind,
        AssetIntelPivotKind::CompanyName | AssetIntelPivotKind::Brand
    ) && !identity_match
    {
        return Err("INTEL_PIVOT_OUTSIDE_FROZEN_COMPANY_IDENTITY");
    }
    let observed = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1 FROM target_intel_asset_observations
                WHERE operation_id=$1 AND organization_id=$2
                  AND asset_kind=$3
                  AND lower(btrim(canonical_value))=lower(btrim($4))
           )"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(pivot.kind.as_str())
    .bind(&pivot.value)
    .fetch_one(pool)
    .await
    .map_err(|_| "TARGET_INTEL_OBSERVATION_FRONTIER_READ_FAILED")?;
    let targets = sqlx::query_as::<_, (String, String)>(
        r#"SELECT target_type::text,value FROM targets
            WHERE organization_id=$1 AND scope::text='in'
            ORDER BY created_at,id"#,
    )
    .bind(organization_id)
    .fetch_all(pool)
    .await
    .map_err(|_| "TARGET_INTEL_SCOPE_READ_FAILED")?;
    let formal_target_match = targets
        .iter()
        .any(|(target_type, value)| formal_target_matches_pivot(target_type, value, pivot));
    classify_semantic_pivot_authorization(
        pivot.kind,
        intent,
        identity_match,
        observed,
        formal_target_match,
    )
}

fn requested_domain_within_authorized_hosts(
    requested: &str,
    authorized_hosts: &[String],
) -> Option<String> {
    let requested = canonical_asset_key(requested)?;
    if requested.class != AssetClass::Domain {
        return None;
    }
    authorized_hosts
        .iter()
        .filter_map(|host| {
            let wildcard = host.trim().starts_with("*.");
            canonical_asset_key(host.trim().trim_start_matches("*.")).map(|key| (key, wildcard))
        })
        .filter(|(host, _)| host.class == AssetClass::Domain)
        .any(|(host, wildcard)| {
            (!wildcard && requested.key == host.key)
                || (requested.key != host.key
                    && requested
                        .key
                        .strip_suffix(&host.key)
                        .is_some_and(|prefix| prefix.ends_with('.')))
        })
        .then_some(requested.key)
}

/// Resolve + IDOR-check the org, run the requested passive phase, and shape the
/// agent-facing result. Shared by both tool impls.
fn candidate_queue_enabled_for_phase(phase: PassiveIntelPhase) -> bool {
    matches!(phase, PassiveIntelPhase::Subsidiaries)
}

fn execution_envelope_complete(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let Some(budget) = object.get("actual_budget").and_then(Value::as_object) else {
        return false;
    };
    let Some(required) = budget.get("required_axes").and_then(Value::as_array) else {
        return false;
    };
    let Some(observed) = budget.get("observed_axes").and_then(Value::as_object) else {
        return false;
    };
    object
        .get("destination_policy_sealed")
        .and_then(Value::as_bool)
        == Some(true)
        && object.get("coverage_extent").and_then(Value::as_str) == Some("complete")
        && object.get("residual_code").is_none_or(Value::is_null)
        && object
            .get("network_hops")
            .and_then(Value::as_array)
            .is_some_and(|hops| !hops.is_empty())
        && required
            .iter()
            .filter_map(Value::as_str)
            .all(|axis| observed.get(axis).is_some_and(Value::is_number))
}

fn evidence_contains_complete_execution(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(evidence_contains_complete_execution),
        Value::Object(object) => {
            object
                .get("toolTruthExecution")
                .or_else(|| object.get("tool_truth_execution"))
                .is_some_and(execution_envelope_complete)
                || object.values().any(evidence_contains_complete_execution)
        }
        _ => false,
    }
}

fn instrumented_provider_ids(evidence: &[Value]) -> std::collections::BTreeSet<String> {
    evidence
        .iter()
        .filter(|value| evidence_contains_complete_execution(value))
        .filter_map(|value| value.get("provider").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

async fn run_phase(
    pool: &Arc<PgPool>,
    tools: &ToolsConfigState,
    args: &Value,
    workspace: &Path,
    phase: PassiveIntelPhase,
    action: &str,
    landing_mode: IntelLandingMode<'_>,
) -> Result<Value> {
    let (receipt_observer, observation_only) = match landing_mode {
        IntelLandingMode::Legacy { receipt_observer } => (receipt_observer, false),
        IntelLandingMode::ObservationOnly => (None, true),
    };
    let id = match args.get("organization_id").and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => return Ok(json!({"error": "'organization_id' is required"})),
    };
    let uid: Uuid = match id.parse() {
        Ok(u) => u,
        Err(_) => return Ok(json!({"error": format!("invalid organization_id: {id}")})),
    };
    let uid = match resolve_bound_organization_id(
        uid,
        golish_core::current_agent_tool_context().and_then(|context| context.organization_id),
    ) {
        Ok(uid) => uid,
        Err(error) => return Ok(json!({"error": error})),
    };
    // IDOR guard (AGENTS.md I2): the org must belong to this project (or be a
    // legacy global row, project_path = '').
    match golish_db::repo::organizations::get_one(pool.as_ref(), uid).await {
        Ok(Some(o)) if organization_visible_in_workspace(&o.project_path, workspace) => {}
        Ok(_) => return Ok(json!({"error": "organization not found in this project"})),
        Err(e) => return Ok(json!({"error": e.to_string()})),
    }

    let (semantic_pivot, semantic_authorization) = match args.get("pivot") {
        Some(value) if !value.is_null() => {
            let intent: IntelSearchIntent = match args
                .get("intent")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
            {
                Some(intent) => intent,
                None => return Ok(json!({"error": "INTEL_SEARCH_INTENT_INVALID"})),
            };
            let raw: AssetIntelPivot = match serde_json::from_value(value.clone()) {
                Ok(pivot) => pivot,
                Err(_) => return Ok(json!({"error": "INTEL_PIVOT_KIND_INVALID"})),
            };
            let pivot = match AssetIntelPivot::parse(raw.kind, raw.value) {
                Ok(pivot) => pivot,
                Err(error) => return Ok(json!({"error": error.to_string()})),
            };
            let authorization =
                match semantic_pivot_authorized(pool.as_ref(), uid, &pivot, intent).await {
                    Ok(authorization) => authorization,
                    Err(code) => return Ok(json!({"error": code})),
                };
            (Some(pivot), Some(authorization))
        }
        _ => (None, None),
    };
    // Scope knobs (only the subsidiaries tool sends these; enrich omits them so
    // they parse to None and behaviour is unchanged).
    let min_ownership_percent = args
        .get("min_ownership_percent")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let include_branches = args.get("include_branches").and_then(|v| v.as_bool());
    // b1 (design 2026-06-24): optional targeted domain-keyed repair
    // (recon_map_assets only). None = normal company-name survey; the asset-intel
    // facade may auto-expand discovered owned apexes after that first run.
    let requested_domain = args
        .get("domain")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let domain = if let Some(pivot) = semantic_pivot.as_ref() {
        matches!(
            pivot.kind,
            AssetIntelPivotKind::Domain | AssetIntelPivotKind::Hostname
        )
        .then(|| pivot.value.clone())
    } else if let Some(requested_domain) = requested_domain {
        let authorized_hosts =
            match crate::asset_intel::authorized_domain_scope_hosts(pool.as_ref(), uid).await {
                Ok(hosts) => hosts,
                Err(error) => return Ok(json!({"error": error.to_string()})),
            };
        match requested_domain_within_authorized_hosts(&requested_domain, &authorized_hosts) {
            Some(domain) => Some(domain),
            None => {
                return Ok(json!({
                    "error": "domain is outside this organization's authorized target roots"
                }))
            }
        }
    } else {
        None
    };
    let config = AssetIntelHydrateConfig {
        min_ownership_percent,
        depth: None,
        include_branches,
        create_candidates: Some(candidate_queue_enabled_for_phase(phase)),
        domain,
        semantic_pivot,
    };

    let receipt_session = if observation_only {
        None
    } else if let Some(observer) = receipt_observer {
        let pentest_config = tools.0.get().await;
        observer
            .begin(TargetIntelReceiptBegin {
                capability: "intel.collect_passive_assets".to_string(),
                endpoints: target_intel_provider_endpoints(
                    pentest_config.controlled_fixture_intel_transport.as_ref(),
                ),
            })
            .await?
    } else {
        None
    };
    if let Some(replay) = receipt_session
        .as_ref()
        .and_then(|session| session.replay_result.clone())
    {
        let mut replay = replay;
        if let Some(object) = replay.as_object_mut() {
            object.insert("action".to_string(), json!(action));
            object.insert("tool_truth_replayed".to_string(), Value::Bool(true));
        }
        return Ok(replay);
    }

    let run = if observation_only {
        run_passive_intel_observation_only(Arc::clone(pool), tools.clone(), uid, phase, config)
            .await
    } else {
        run_passive_intel(Arc::clone(pool), tools.clone(), uid, phase, config).await
    };
    match run {
        Ok(summary) => {
            if let (Some(observer), Some(session)) = (receipt_observer, receipt_session) {
                let instrumented = instrumented_provider_ids(&summary.tool_truth_provider_evidence);
                let mut by_technique =
                    std::collections::BTreeMap::<String, Vec<(&str, &str)>>::new();
                for status in &summary.technique_status {
                    by_technique
                        .entry(status.technique.clone())
                        .or_default()
                        .push((status.source.as_str(), status.status.as_str()));
                }
                let technique_observations = by_technique
                    .into_iter()
                    .map(|(technique, statuses)| {
                        let observed = statuses
                            .iter()
                            .filter(|(source, _)| instrumented.contains(*source))
                            .map(|(_, status)| *status)
                            .collect::<Vec<_>>();
                        let observation_state = if observed.contains(&"found") {
                            "found"
                        } else if observed.contains(&"empty") {
                            "no_match"
                        } else {
                            "indeterminate"
                        };
                        TargetIntelTechniqueObservation {
                            technique,
                            observation_state: observation_state.to_string(),
                        }
                    })
                    .collect();
                let typed_landing = serde_json::to_value(&summary).unwrap_or_else(|_| json!({}));
                observer
                    .finalize(
                        session,
                        TargetIntelReceiptFinalization {
                            provider_evidence: summary.tool_truth_provider_evidence.clone(),
                            technique_observations,
                            typed_landing,
                            failure_reason_code: summary.error.clone(),
                        },
                    )
                    .await?;
            }
            let semantic_landing = if observation_only {
                let pivot = args
                    .get("pivot")
                    .and_then(Value::as_object)
                    .ok_or_else(|| {
                        anyhow::anyhow!("TARGET_INTEL_SEMANTIC_PIVOT_CONTEXT_MISSING")
                    })?;
                let kind = pivot
                    .get("kind")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_SEMANTIC_PIVOT_KIND_MISSING"))?;
                let value = pivot
                    .get("value")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_SEMANTIC_PIVOT_VALUE_MISSING"))?;
                let semantic_authorization = semantic_authorization.ok_or_else(|| {
                    anyhow::anyhow!("TARGET_INTEL_SEMANTIC_AUTHORIZATION_MISSING")
                })?;
                Some(
                    land_semantic_goal_observations(
                        pool.as_ref(),
                        &summary,
                        SemanticPivotLandingContext {
                            kind,
                            value,
                            authorization_basis: semantic_authorization.authorization_basis(),
                            has_scope_authority: semantic_authorization.grants_scope_authority(),
                            candidate_only: semantic_authorization.candidate_only(),
                        },
                        workspace,
                    )
                    .await?,
                )
            } else {
                None
            };
            let mut value = serde_json::to_value(&summary).unwrap_or_else(|_| json!({}));
            if let Some(map) = value.as_object_mut() {
                map.insert("action".to_string(), json!(action));
                if let Some(semantic_landing) = semantic_landing {
                    map.insert("semantic_landing".to_string(), semantic_landing);
                    map.insert("targets".to_string(), json!(0));
                    map.insert(
                        "landing_authority".to_string(),
                        json!("observation_lifecycle_v1"),
                    );
                }
            }
            Ok(value)
        }
        Err(error) => {
            let value = json!({"error": error.to_string()});
            if let (Some(observer), Some(session)) = (receipt_observer, receipt_session) {
                observer
                    .finalize(
                        session,
                        TargetIntelReceiptFinalization {
                            provider_evidence: Vec::new(),
                            technique_observations: Vec::new(),
                            typed_landing: value.clone(),
                            failure_reason_code: Some("target_intel_provider_failed".to_string()),
                        },
                    )
                    .await?;
            }
            Ok(value)
        }
    }
}

/// `recon_discover_subsidiaries` — ENScan enterprise-intel: find subsidiary /
/// affiliate organizations of the engagement subject (red-team intel step 1).
pub struct ReconDiscoverSubsidiariesTool {
    pool: Arc<PgPool>,
    tools: ToolsConfigState,
}

enum IntelLandingMode<'a> {
    Legacy {
        receipt_observer: Option<&'a Arc<dyn TargetIntelReceiptObserver>>,
    },
    ObservationOnly,
}

impl ReconDiscoverSubsidiariesTool {
    pub fn new(pool: Arc<PgPool>, tools: ToolsConfigState) -> Self {
        Self { pool, tools }
    }
}

#[async_trait::async_trait]
impl Tool for ReconDiscoverSubsidiariesTool {
    fn name(&self) -> &'static str {
        "recon_discover_subsidiaries"
    }

    fn description(&self) -> &'static str {
        "Passively discover subsidiary / affiliate organizations of the engagement subject via the enterprise-intel provider (ENScan: 爱企查/天眼查). Use during target_intel for red-team engagements before enriching assets. Writes candidate organizations back to the org for review. Returns a summary with counts and provider ids. Before calling, ask the human (scoping) whether subsidiaries are in scope and at what ownership threshold; pass min_ownership_percent accordingly."
    }

    fn parameters(&self) -> Value {
        subsidiary_intel_parameters()
    }

    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        run_phase(
            &self.pool,
            &self.tools,
            &args,
            workspace,
            PassiveIntelPhase::Subsidiaries,
            "discover_subsidiaries",
            IntelLandingMode::Legacy {
                receipt_observer: None,
            },
        )
        .await
    }
}

/// `recon_map_assets` — cyberspace/intel-provider survey (0.zone / quake / fofa /
/// hunter / shodan / ENScan): collect domains / IPs / ASN / subdomains /
/// certificates / ICP / apps / emails / OSINT, landed to the org profile +
/// target_assets (host↔IP pairs carry the surveyed real_ip). Replaces the old
/// all-in-one enrich tool; WHOIS is the standalone `recon_lookup_whois` tool.
pub struct ReconMapAssetsTool {
    pool: Arc<PgPool>,
    tools: ToolsConfigState,
    receipt_observer: Option<Arc<dyn TargetIntelReceiptObserver>>,
}

/// Production semantic Target Intel entry point. The model chooses only a
/// typed pivot and intent; provider selection, query compilation, scope and
/// receipt authority remain inside the host.
pub struct ReconSearchIntelTool {
    pool: Arc<PgPool>,
    tools: ToolsConfigState,
    _legacy_receipt_observer: Arc<dyn TargetIntelReceiptObserver>,
}

fn semantic_search_model_result(raw: &Value, args: &Value) -> Value {
    let pivot = args.get("pivot").cloned().unwrap_or(Value::Null);
    let intent = args.get("intent").cloned().unwrap_or(Value::Null);
    if let Some(error) = raw.get("error") {
        return json!({
            "schema": "intel_semantic_search_result.v1",
            "status": "blocked",
            "code": "INTEL_SEMANTIC_SEARCH_FAILED",
            "pivot": pivot,
            "intent": intent,
            "residual": {"kind": "provider_or_landing_error", "detail": error},
        });
    }
    let Some(landing) = raw.get("semantic_landing").and_then(Value::as_object) else {
        return json!({
            "schema": "intel_semantic_search_result.v1",
            "status": "blocked",
            "code": "INTEL_SEMANTIC_LANDING_MISSING",
            "pivot": pivot,
            "intent": intent,
            "residual": {"kind": "host_contract_error"},
        });
    };
    let observations = landing
        .get("observations")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let observation_count = observations.as_array().map_or(0, Vec::len);
    let promoted_count = landing
        .get("promoted_target_refs")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let ambiguous_count = landing
        .get("ambiguous_refs")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let unreachable_count = landing
        .get("unreachable_refs")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    json!({
        "schema": "intel_semantic_search_result.v1",
        "status": if observation_count == 0 { "empty" } else { "succeeded" },
        "pivot": pivot,
        "intent": intent,
        "landing_authority": "observation_lifecycle_v1",
        "observations": observations,
        "discovered_pivots": landing.get("discovered_pivots").cloned().unwrap_or_else(|| json!([])),
        "observation_refs": landing.get("observation_refs").cloned().unwrap_or_else(|| json!([])),
        "promoted_target_refs": landing.get("promoted_target_refs").cloned().unwrap_or_else(|| json!([])),
        "ambiguous_refs": landing.get("ambiguous_refs").cloned().unwrap_or_else(|| json!([])),
        "unreachable_refs": landing.get("unreachable_refs").cloned().unwrap_or_else(|| json!([])),
        "evidence_ids": landing.get("evidence_ids").cloned().unwrap_or_else(|| json!([])),
        "counts": {
            "observations": observation_count,
            "promoted_targets": promoted_count,
            "ambiguous": ambiguous_count,
            "unreachable": unreachable_count,
        },
    })
}

impl ReconSearchIntelTool {
    pub fn new(
        pool: Arc<PgPool>,
        tools: ToolsConfigState,
        receipt_observer: Arc<dyn TargetIntelReceiptObserver>,
    ) -> Self {
        Self {
            pool,
            tools,
            _legacy_receipt_observer: receipt_observer,
        }
    }
}

#[async_trait::async_trait]
impl Tool for ReconSearchIntelTool {
    fn name(&self) -> &'static str {
        "recon_search_intel"
    }

    fn description(&self) -> &'static str {
        "Search corporate asset intelligence from a host-compiled semantic pivot. Supply only organization_id, pivot {kind,value}, and intent. The host accepts exact frozen facts and novel Goal hypotheses, records their distinct authorization basis, selects registered provider adapters, escapes provider syntax, and records artifacts/evidence. A hypothesis is only a passive search term: it grants no scope, ownership, reachability, promotion, or later active-scan authority. Use returned observations and discovered pivots to adapt the plan; never write provider DSL."
    }

    fn parameters(&self) -> Value {
        recon_search_intel_parameters()
    }

    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        let Some(kind) = args
            .get("pivot")
            .and_then(Value::as_object)
            .and_then(|pivot| pivot.get("kind"))
            .and_then(Value::as_str)
        else {
            return Ok(json!({"error": "INTEL_PIVOT_KIND_INVALID"}));
        };
        let Some(value) = args
            .get("pivot")
            .and_then(Value::as_object)
            .and_then(|pivot| pivot.get("value"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(json!({"error": "INTEL_PIVOT_VALUE_INVALID"}));
        };
        let parsed_kind: AssetIntelPivotKind = match serde_json::from_value(json!(kind)) {
            Ok(kind) => kind,
            Err(_) => return Ok(json!({"error": "INTEL_PIVOT_KIND_INVALID"})),
        };
        let pivot = match AssetIntelPivot::parse(parsed_kind, value) {
            Ok(pivot) => pivot,
            Err(error) => return Ok(json!({"error": error.to_string()})),
        };
        let mut phase_args = json!({
            "organization_id": args.get("organization_id").cloned().unwrap_or(Value::Null),
            "pivot": pivot,
            "intent": args.get("intent").cloned().unwrap_or(Value::Null),
        });
        match parsed_kind {
            AssetIntelPivotKind::Domain | AssetIntelPivotKind::Hostname => {
                phase_args["domain"] = json!(value)
            }
            _ => {}
        }
        let result = run_phase(
            &self.pool,
            &self.tools,
            &phase_args,
            workspace,
            PassiveIntelPhase::Enrich,
            "search_intel",
            IntelLandingMode::ObservationOnly,
        )
        .await?;
        Ok(semantic_search_model_result(&result, &args))
    }
}

impl ReconMapAssetsTool {
    pub fn new(pool: Arc<PgPool>, tools: ToolsConfigState) -> Self {
        Self {
            pool,
            tools,
            receipt_observer: None,
        }
    }

    pub fn with_receipt_observer(
        pool: Arc<PgPool>,
        tools: ToolsConfigState,
        receipt_observer: Arc<dyn TargetIntelReceiptObserver>,
    ) -> Self {
        Self {
            pool,
            tools,
            receipt_observer: Some(receipt_observer),
        }
    }
}

#[async_trait::async_trait]
impl Tool for ReconMapAssetsTool {
    fn name(&self) -> &'static str {
        "recon_map_assets"
    }

    fn description(&self) -> &'static str {
        "Survey an organization's external footprint via every configured cyberspace/intel provider (0.zone / quake / fofa / hunter / shodan / ENScan). The backend canonicalizes and deduplicates this invocation's domains and IPs, writes them directly as organization-bound Targets, preserves every hostname↔IP edge in dns_records, and lands service/subdomain relationships; no manual target-candidate approval step is involved. Organization profile fields (ASN, certificates, ICP, apps/mini-programs, exposed emails, OSINT) are updated in the same zero-touch pass. Use during target_intel after the engagement subject is confirmed. Optional `domain` is only for a targeted repair within a pre-authorized query root; provider-discovered Targets do not recursively expand query scope. WHOIS is a separate tool (recon_lookup_whois). Returns separate observation and durable landing counts."
    }

    fn parameters(&self) -> Value {
        map_assets_parameters()
    }

    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        run_phase(
            &self.pool,
            &self.tools,
            &args,
            workspace,
            PassiveIntelPhase::Enrich,
            "map_assets",
            IntelLandingMode::Legacy {
                receipt_observer: self.receipt_observer.as_ref(),
            },
        )
        .await
    }
}

/// `recon_lookup_whois` — standalone WHOIS-via-RDAP lookup for an organization,
/// once per org across its registrable domains, landing to `organizations.whois`
/// (the target_intel WHOIS coverage cell). Zero-touch HTTP. Split out of the old
/// all-in-one enrich tool. (plan 2026-06-18-slim-enrich)
pub struct ReconLookupWhoisTool {
    pool: Arc<PgPool>,
    tools: Option<ToolsConfigState>,
    receipt_observer: Option<Arc<dyn TargetIntelReceiptObserver>>,
}

impl ReconLookupWhoisTool {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self {
            pool,
            tools: None,
            receipt_observer: None,
        }
    }

    pub fn with_receipt_observer(
        pool: Arc<PgPool>,
        tools: ToolsConfigState,
        receipt_observer: Arc<dyn TargetIntelReceiptObserver>,
    ) -> Self {
        Self {
            pool,
            tools: Some(tools),
            receipt_observer: Some(receipt_observer),
        }
    }
}

#[async_trait::async_trait]
impl Tool for ReconLookupWhoisTool {
    fn name(&self) -> &'static str {
        "recon_lookup_whois"
    }

    fn description(&self) -> &'static str {
        "Look up fresh domain registration data (WHOIS via RDAP) for an organization across its registrable domains and land it to organizations.whois (the target_intel WHOIS coverage cell). Zero-touch HTTP. Use during target_intel. Args: organization_id. Returns typed whois_status=found|empty|error|blocked plus attempt counts; an old stored value never substitutes for this run's request."
    }

    fn parameters(&self) -> Value {
        passive_intel_parameters("to look up WHOIS for")
    }

    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        let org_id = match args.get("organization_id").and_then(Value::as_str) {
            Some(s) => match Uuid::parse_str(s.trim()) {
                Ok(id) => id,
                Err(_) => return Ok(json!({"error": "'organization_id' must be a valid UUID"})),
            },
            None => return Ok(json!({"error": "'organization_id' is required"})),
        };
        let org_id = match resolve_bound_organization_id(
            org_id,
            golish_core::current_agent_tool_context().and_then(|context| context.organization_id),
        ) {
            Ok(org_id) => org_id,
            Err(error) => return Ok(json!({"error": error})),
        };
        // Same project ownership guard as recon_map_assets. WHOIS performs a real
        // external request, so resolving an arbitrary cross-workspace org id must
        // fail closed before RDAP is invoked.
        let org = match golish_db::repo::organizations::get_one(self.pool.as_ref(), org_id).await {
            Ok(Some(org)) if organization_visible_in_workspace(&org.project_path, workspace) => org,
            Ok(_) => return Ok(json!({"error": "organization not found in this project"})),
            Err(e) => return Ok(json!({"error": e.to_string()})),
        };
        let authorized_hosts =
            match crate::asset_intel::whois_domain_scope_hosts(self.pool.as_ref(), org_id).await {
                Ok(hosts) => hosts,
                Err(error) => return Ok(json!({"error": error.to_string()})),
            };
        if let Some(tools) = self.tools.as_ref() {
            if tools
                .0
                .get()
                .await
                .controlled_fixture_intel_transport
                .is_some()
            {
                return Ok(json!({
                    "action": "lookup_whois",
                    "organization_id": org_id.to_string(),
                    "whois_status": "blocked",
                    "attempted": 0,
                    "succeeded": 0,
                    "reason": "controlled_fixture_external_provider_disabled",
                }));
            }
        }
        let receipt_session = if let Some(observer) = self.receipt_observer.as_ref() {
            observer
                .begin(TargetIntelReceiptBegin {
                    capability: "intel.collect_whois".to_string(),
                    endpoints: target_intel_fixed_provider_endpoints()
                        .into_iter()
                        .filter(|endpoint| endpoint.normalized_host == "rdap.org")
                        .collect(),
                })
                .await?
        } else {
            None
        };
        if let Some(replay) = receipt_session
            .as_ref()
            .and_then(|session| session.replay_result.clone())
        {
            return Ok(replay);
        }
        match crate::organization_recon::land_whois(self.pool.as_ref(), &org, &authorized_hosts)
            .await
        {
            Ok(outcome) => {
                let value = json!({
                    "action": "lookup_whois",
                    "organization_id": org_id.to_string(),
                    "whois_landed": matches!(outcome.state, crate::organization_recon::WhoisLandingState::Found),
                    "whois_status": outcome.state.as_str(),
                    "attempted": outcome.attempted,
                    "succeeded": outcome.succeeded,
                    "reason": outcome.reason,
                });
                if let (Some(observer), Some(session)) =
                    (self.receipt_observer.as_ref(), receipt_session)
                {
                    let observation_state = match outcome.state {
                        crate::organization_recon::WhoisLandingState::Found => "found",
                        crate::organization_recon::WhoisLandingState::Empty => "no_match",
                        crate::organization_recon::WhoisLandingState::Error
                        | crate::organization_recon::WhoisLandingState::Blocked => "indeterminate",
                    };
                    observer
                        .finalize(
                            session,
                            TargetIntelReceiptFinalization {
                                provider_evidence: outcome.tool_truth_provider_evidence,
                                technique_observations: vec![TargetIntelTechniqueObservation {
                                    technique: "GOLISH-INTEL-WHOIS".to_string(),
                                    observation_state: observation_state.to_string(),
                                }],
                                typed_landing: value.clone(),
                                failure_reason_code: (observation_state == "indeterminate")
                                    .then(|| "whois_incomplete".to_string()),
                            },
                        )
                        .await?;
                }
                Ok(value)
            }
            Err(error) => {
                let value = json!({"error": error.to_string()});
                if let (Some(observer), Some(session)) =
                    (self.receipt_observer.as_ref(), receipt_session)
                {
                    observer
                        .finalize(
                            session,
                            TargetIntelReceiptFinalization {
                                provider_evidence: Vec::new(),
                                technique_observations: vec![TargetIntelTechniqueObservation {
                                    technique: "GOLISH-INTEL-WHOIS".to_string(),
                                    observation_state: "indeterminate".to_string(),
                                }],
                                typed_landing: value.clone(),
                                failure_reason_code: Some("whois_provider_failed".to_string()),
                            },
                        )
                        .await?;
                }
                Ok(value)
            }
        }
    }
}

/// `recon_lookup_company` — scoping 纠名 step 1 (设计 2026-06-13-engagement-
/// scoping-fanout §6.2): resolve a raw / colloquial company name to canonical
/// registered names via the enterprise-intel lookup runtime (ENScan 企查查,
/// `company-lookup-json`). Read-only: queries the business registry, never
/// touches the target, writes nothing to organizations.
pub struct ReconLookupCompanyTool {
    pool: Arc<PgPool>,
    tools: ToolsConfigState,
}

impl ReconLookupCompanyTool {
    pub fn new(pool: Arc<PgPool>, tools: ToolsConfigState) -> Self {
        Self { pool, tools }
    }
}

/// JSON schema for `recon_lookup_company` (free function for unit testing).
fn lookup_company_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "keyword": {
                "type": "string",
                "description": "Raw company name to normalize (e.g. pasted by the user). The lookup queries the business registry and returns canonical registered names."
            },
            "limit": {
                "type": "integer",
                "description": "Max matches to return (default 5, hard cap 25). The first match is the highest-confidence canonical name."
            },
            "selected_candidate_id": {
                "type": "string",
                "description": "Host-issued candidate id selected by the human after an ambiguous result. Omit for the first lookup."
            }
        },
        "required": ["keyword"]
    })
}

#[async_trait::async_trait]
impl Tool for ReconLookupCompanyTool {
    fn name(&self) -> &'static str {
        "recon_lookup_company"
    }

    fn description(&self) -> &'static str {
        "Scoping company resolver. The host runs the ordered enterprise lookup policy and freezes an immutable Company Identity receipt. One strong unique match is confirmed deterministically; ambiguous matches return host-issued candidate ids and require exactly one ask_human choice, then call this tool again with selected_candidate_id. Never select the first result by convention and never create an organization from free text. This tool performs no asset discovery or target probing."
    }

    fn parameters(&self) -> Value {
        lookup_company_parameters()
    }

    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        let keyword = match args.get("keyword").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => return Ok(json!({"error": "'keyword' is required"})),
        };
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .or(Some(5));
        let selected_candidate_id = args
            .get("selected_candidate_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let pentest_config = self.tools.0.get().await;
        match lookup_company_matches(&self.pool, &pentest_config, &keyword, &[], limit).await {
            Ok(result) => match freeze_company_lookup_result(
                self.pool.as_ref(),
                workspace,
                &keyword,
                selected_candidate_id,
                &result,
            )
            .await
            {
                Ok(identity) => Ok(json!({
                    "action": "lookup_company",
                    "keyword": keyword,
                    "match_count": result.matches.len(),
                    "matches": result.matches,
                    "provider_status": result.provider_status,
                    "company_identity": identity,
                })),
                Err(error) => Ok(json!({"error": error.to_string()})),
            },
            Err(e) => Ok(json!({"error": e.to_string()})),
        }
    }
}

/// `recon_list_providers` — list the passive asset-intel providers and whether
/// each is currently usable (its credential is configured). Call this during
/// target_intel BEFORE the discover/enrich tools so the AI only invokes
/// providers it can actually run, and records the rest as blocked (no
/// credential) rather than fabricating coverage (AGENTS.md I8).
pub struct ReconListProvidersTool {
    pool: Arc<PgPool>,
    tools: ToolsConfigState,
}

impl ReconListProvidersTool {
    pub fn new(pool: Arc<PgPool>, tools: ToolsConfigState) -> Self {
        Self { pool, tools }
    }
}

#[async_trait::async_trait]
impl Tool for ReconListProvidersTool {
    fn name(&self) -> &'static str {
        "recon_list_providers"
    }

    fn description(&self) -> &'static str {
        "List the passive asset-intel providers (ENScan subsidiary discovery, 0.zone/quake/… enrichment) and whether each is currently usable, i.e. its credential/integration is configured. Call this FIRST during target_intel, before recon_discover_subsidiaries / recon_map_assets, so you only invoke providers that can actually run; for intel techniques with no available provider, record coverage as blocked (no credential) — never fabricate. Read-only: never runs a provider, never touches the target. Returns each provider's id, phase (subsidiaries/enrich), capabilities, available flag, and reason."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {}, "required": [] })
    }

    async fn execute(&self, _args: Value, _workspace: &Path) -> Result<Value> {
        match list_provider_availability(Arc::clone(&self.pool), self.tools.clone()).await {
            Ok(providers) => {
                let available = providers.iter().filter(|p| p.available).count();
                Ok(json!({
                    "providers": providers,
                    "available_count": available,
                    "total_count": providers.len(),
                }))
            }
            Err(e) => Ok(json!({ "error": e.to_string() })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_requires_organization_id() {
        let p = passive_intel_parameters("to enrich assets for");
        let required = p["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "organization_id"));
        assert!(p["properties"].get("organization_id").is_some());
    }

    #[test]
    fn recon_search_intel_schema_is_closed_at_every_object_level() {
        let schema = recon_search_intel_parameters();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["pivot"]["additionalProperties"], false);
        assert!(schema["properties"].get("provider_id").is_none());
        assert!(schema["properties"]["pivot"]["properties"]
            .get("dsl")
            .is_none());
    }

    #[test]
    fn recon_search_intel_model_result_exposes_semantic_observations_not_legacy_axes() {
        let raw = json!({
            "techniqueStatus": [
                {"technique": "GOLISH-INTEL-DNS", "status": "error"},
                {"technique": "GOLISH-INTEL-SUBDOMAIN", "status": "error"}
            ],
            "semantic_landing": {
                "observations": [{
                    "observation_id": Uuid::nil(),
                    "asset_kind": "domain",
                    "canonical_value": "moresec.cn",
                    "provider_id": "fixture",
                    "attribution": "owned",
                    "reachability": "reachable",
                    "promotion_target_id": Uuid::nil(),
                    "evidence_ids": [41, 42]
                }],
                "observation_refs": [Uuid::nil()],
                "promoted_target_refs": [Uuid::nil()],
                "ambiguous_refs": [],
                "unreachable_refs": [],
                "evidence_ids": [41, 42],
                "discovered_pivots": [{"kind": "asn", "value": "AS4134"}]
            }
        });
        let args = json!({
            "pivot": {"kind": "company_name", "value": "杭州默安科技有限公司"},
            "intent": "discover_related_assets"
        });

        let result = semantic_search_model_result(&raw, &args);
        assert_eq!(result["schema"], "intel_semantic_search_result.v1");
        assert_eq!(result["status"], "succeeded");
        assert_eq!(result["evidence_ids"], json!([41, 42]));
        assert_eq!(
            result["discovered_pivots"],
            json!([{"kind": "asn", "value": "AS4134"}])
        );
        let serialized = result.to_string();
        assert!(!serialized.contains("GOLISH-INTEL-"));
        assert!(!serialized.contains("techniqueStatus"));
    }

    #[test]
    fn ungrounded_network_hypotheses_and_enrichment_are_rejected() {
        assert_eq!(
            classify_semantic_pivot_authorization(
                AssetIntelPivotKind::Asn,
                IntelSearchIntent::DiscoverRelatedAssets,
                false,
                false,
                false,
            ),
            Err("INTEL_PIVOT_NOT_IN_AUTHORIZED_FRONTIER")
        );
        assert_eq!(
            classify_semantic_pivot_authorization(
                AssetIntelPivotKind::Domain,
                IntelSearchIntent::EnrichKnownAsset,
                false,
                None,
                false,
            ),
            Err("INTEL_PIVOT_NOT_IN_AUTHORIZED_FRONTIER")
        );
    }

    #[test]
    fn formal_target_authority_requires_matching_target_type() {
        let pivot = AssetIntelPivot::parse(AssetIntelPivotKind::Ip, "1.1.1.1").unwrap();
        assert!(formal_target_matches_pivot("ip", "1.1.1.1", &pivot));
        assert!(!formal_target_matches_pivot("domain", "1.1.1.1", &pivot));
        assert!(!formal_target_matches_pivot(
            "url",
            "https://1.1.1.1",
            &pivot
        ));
    }

    #[test]
    fn pivots_discovered_from_candidate_only_observations_stay_candidate_only() {
        let authorization = classify_semantic_pivot_authorization(
            AssetIntelPivotKind::Asn,
            IntelSearchIntent::DiscoverRelatedAssets,
            false,
            Some(true),
            false,
        )
        .expect("a provider-discovered ASN remains searchable as a bounded observation pivot");
        assert_eq!(
            authorization,
            SemanticPivotAuthorization::PriorObservationCandidateOnly
        );
        assert!(authorization.candidate_only());
        assert!(!authorization.grants_scope_authority());
    }

    #[test]
    fn map_assets_schema_has_optional_domain() {
        // b1 (design 2026-06-24): recon_map_assets exposes an optional `domain`
        // knob; organization_id stays required.
        let p = map_assets_parameters();
        let required = p["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "organization_id"));
        assert!(!required.iter().any(|r| r == "domain"));
        assert!(p["properties"].get("domain").is_some());
    }

    #[test]
    fn asset_map_disables_target_candidate_queue_but_subsidiary_review_keeps_it() {
        assert!(!candidate_queue_enabled_for_phase(
            PassiveIntelPhase::Enrich
        ));
        assert!(candidate_queue_enabled_for_phase(
            PassiveIntelPhase::Subsidiaries
        ));
    }

    #[test]
    fn whois_project_guard_rejects_cross_workspace_org() {
        let workspace = Path::new("/tmp/current-project");
        assert!(organization_visible_in_workspace(
            "/tmp/current-project",
            workspace
        ));
        assert!(organization_visible_in_workspace("", workspace));
        assert!(
            !organization_visible_in_workspace("/tmp/other-project", workspace),
            "WHOIS must not load an organization owned by another workspace"
        );
    }

    #[test]
    fn stage_bound_organization_cannot_be_overridden_by_model_args() {
        let bound = Uuid::new_v4();
        let sibling = Uuid::new_v4();
        assert_eq!(resolve_bound_organization_id(bound, Some(bound)), Ok(bound));
        assert!(resolve_bound_organization_id(sibling, Some(bound))
            .expect_err("sibling org override must be rejected")
            .contains("current stage-run organization"));
        assert_eq!(
            resolve_bound_organization_id(sibling, None),
            Ok(sibling),
            "standalone GUI/direct calls keep the project-scoped legacy path"
        );
    }

    #[test]
    fn targeted_domain_repair_never_expands_upward_or_to_third_party() {
        let roots = vec![
            "www.moresec.cn".to_string(),
            "api.example.com".to_string(),
            "*.wild.moresec.cn".to_string(),
        ];
        assert_eq!(
            requested_domain_within_authorized_hosts("WWW.MoreSec.CN.", &roots).as_deref(),
            Some("www.moresec.cn")
        );
        assert_eq!(
            requested_domain_within_authorized_hosts("a.www.moresec.cn", &roots).as_deref(),
            Some("a.www.moresec.cn")
        );
        assert_eq!(
            requested_domain_within_authorized_hosts("moresec.cn", &roots),
            None,
            "an approved child host must not authorize its parent/apex"
        );
        assert_eq!(
            requested_domain_within_authorized_hosts("cdn.vendor.net", &roots),
            None
        );
        assert_eq!(
            requested_domain_within_authorized_hosts("wild.moresec.cn", &roots),
            None,
            "wildcard scope authorizes strict children, not its base/apex"
        );
        assert_eq!(
            requested_domain_within_authorized_hosts("a.wild.moresec.cn", &roots).as_deref(),
            Some("a.wild.moresec.cn")
        );
    }

    #[test]
    fn lookup_schema_requires_keyword_only() {
        let p = lookup_company_parameters();
        let required = p["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert!(required.iter().any(|r| r == "keyword"));
        assert!(p["properties"].get("limit").is_some());
    }
}
