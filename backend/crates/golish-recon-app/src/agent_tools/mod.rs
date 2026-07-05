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

use crate::asset_intel::ToolsConfigState;
use crate::asset_intel::{
    list_provider_availability, lookup_company_matches, run_passive_intel, AssetIntelHydrateConfig,
    PassiveIntelPhase,
};

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

/// Resolve + IDOR-check the org, run the requested passive phase, and shape the
/// agent-facing result. Shared by both tool impls.
async fn run_phase(
    pool: &Arc<PgPool>,
    tools: &ToolsConfigState,
    args: &Value,
    workspace: &Path,
    phase: PassiveIntelPhase,
    action: &str,
) -> Result<Value> {
    let id = match args.get("organization_id").and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => return Ok(json!({"error": "'organization_id' is required"})),
    };
    let uid: Uuid = match id.parse() {
        Ok(u) => u,
        Err(_) => return Ok(json!({"error": format!("invalid organization_id: {id}")})),
    };
    let project_path = workspace.to_string_lossy().to_string();

    // IDOR guard (AGENTS.md I2): the org must belong to this project (or be a
    // legacy global row, project_path = '').
    match golish_db::repo::organizations::get_one(pool.as_ref(), uid).await {
        Ok(Some(o)) if o.project_path == project_path || o.project_path.is_empty() => {}
        Ok(_) => return Ok(json!({"error": "organization not found in this project"})),
        Err(e) => return Ok(json!({"error": e.to_string()})),
    }

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
    let domain = args
        .get("domain")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let config = AssetIntelHydrateConfig {
        min_ownership_percent,
        depth: None,
        include_branches,
        create_candidates: Some(true),
        domain,
    };

    match run_passive_intel(Arc::clone(pool), tools.clone(), uid, phase, config).await {
        Ok(summary) => {
            let mut value = serde_json::to_value(&summary).unwrap_or_else(|_| json!({}));
            if let Some(map) = value.as_object_mut() {
                map.insert("action".to_string(), json!(action));
            }
            Ok(value)
        }
        Err(e) => Ok(json!({"error": e.to_string()})),
    }
}

/// `recon_discover_subsidiaries` — ENScan enterprise-intel: find subsidiary /
/// affiliate organizations of the engagement subject (red-team intel step 1).
pub struct ReconDiscoverSubsidiariesTool {
    pool: Arc<PgPool>,
    tools: ToolsConfigState,
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
}

impl ReconMapAssetsTool {
    pub fn new(pool: Arc<PgPool>, tools: ToolsConfigState) -> Self {
        Self { pool, tools }
    }
}

#[async_trait::async_trait]
impl Tool for ReconMapAssetsTool {
    fn name(&self) -> &'static str {
        "recon_map_assets"
    }

    fn description(&self) -> &'static str {
        "Survey an organization's external footprint via cyberspace/intel providers (0.zone / quake / fofa / hunter / shodan / ENScan): domains, IP ranges, ASN, subdomains, certificates, ICP records, apps/mini-programs, exposed emails, and OSINT — landed to the org profile + target_assets (host↔IP pairs carry the surveyed real_ip). Zero-touch. Use during target_intel after the engagement subject is confirmed. The normal org/company survey automatically expands bounded owned apex domains it discovers using DOMAIN-keyed provider templates; optional `domain` is only for targeted repair/manual supplement. WHOIS is a separate tool (recon_lookup_whois). Returns a summary with counts, provider ids, and domainExpansions when apex expansion ran."
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
}

impl ReconLookupWhoisTool {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl Tool for ReconLookupWhoisTool {
    fn name(&self) -> &'static str {
        "recon_lookup_whois"
    }

    fn description(&self) -> &'static str {
        "Look up domain registration (WHOIS via RDAP) for an organization — once per org across its registrable domains — and land it to organizations.whois (the target_intel WHOIS coverage cell). Zero-touch HTTP, best-effort (only fills when empty). Use during target_intel. Args: organization_id. Returns whether a whois record landed."
    }

    fn parameters(&self) -> Value {
        passive_intel_parameters("to look up WHOIS for")
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let org_id = match args.get("organization_id").and_then(Value::as_str) {
            Some(s) => match Uuid::parse_str(s.trim()) {
                Ok(id) => id,
                Err(_) => return Ok(json!({"error": "'organization_id' must be a valid UUID"})),
            },
            None => return Ok(json!({"error": "'organization_id' is required"})),
        };
        let org = match golish_db::repo::organizations::get_one(self.pool.as_ref(), org_id).await {
            Ok(Some(org)) => org,
            Ok(None) => return Ok(json!({"error": format!("organization {org_id} not found")})),
            Err(e) => return Ok(json!({"error": e.to_string()})),
        };
        match crate::organization_recon::land_whois(self.pool.as_ref(), &org).await {
            Ok(landed) => Ok(json!({
                "action": "lookup_whois",
                "organization_id": org_id.to_string(),
                "whois_landed": landed,
            })),
            Err(e) => Ok(json!({"error": e.to_string()})),
        }
    }
}

/// `recon_lookup_company` — scoping 纠名 step 1 (设计 2026-06-13-engagement-
/// scoping-fanout §6.2): resolve a raw / colloquial company name to canonical
/// registered names via the enterprise-intel lookup runtime (ENScan 企查查,
/// `company-lookup-json`). Read-only: queries the business registry, never
/// touches the target, writes nothing to organizations.
pub struct ReconLookupCompanyTool {
    tools: ToolsConfigState,
}

impl ReconLookupCompanyTool {
    pub fn new(tools: ToolsConfigState) -> Self {
        Self { tools }
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
        "Scoping STEP 1 — resolve a raw company name to its canonical registered name (以企查查等企业登记数据为准) BEFORE creating organizations. Returns canonical matches with credit_code, legal_representative and confidence (sorted desc). Pick the best match (usually the first) and use its exact `name` for manage_organizations create / create_batch. If no provider credential is configured or there is no match, record that company as 纠名失败/待人工 and ask the user — never guess or invent a canonical name. Read-only: never probes the target, writes nothing."
    }

    fn parameters(&self) -> Value {
        lookup_company_parameters()
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let keyword = match args.get("keyword").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => return Ok(json!({"error": "'keyword' is required"})),
        };
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .or(Some(5));

        let pentest_config = self.tools.0.get().await;
        match lookup_company_matches(&pentest_config, &keyword, &[], limit).await {
            Ok(result) => Ok(json!({
                "action": "lookup_company",
                "keyword": keyword,
                "match_count": result.matches.len(),
                "matches": result.matches,
                "provider_status": result.provider_status,
            })),
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
    fn lookup_schema_requires_keyword_only() {
        let p = lookup_company_parameters();
        let required = p["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert!(required.iter().any(|r| r == "keyword"));
        assert!(p["properties"].get("limit").is_some());
    }
}
