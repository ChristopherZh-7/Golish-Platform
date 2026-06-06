//! Agent tools that let the harness `target_intel` stage drive the passive
//! asset-intel engine directly (设计 2026-06-06-intel-stage-ai-driven-per-mode
//! §3.3). These wrap [`crate::asset_intel::run_passive_intel`] so the AI — not a
//! GUI button — performs subsidiary discovery (`recon_discover_subsidiaries`,
//! ENScan) and field enrichment (`recon_enrich_assets`, 0.zone / quake / …).
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
    list_provider_availability, run_passive_intel, AssetIntelHydrateConfig, PassiveIntelPhase,
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

    match run_passive_intel(
        Arc::clone(pool),
        tools.clone(),
        uid,
        phase,
        AssetIntelHydrateConfig::default(),
    )
    .await
    {
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
        "Passively discover subsidiary / affiliate organizations of the engagement subject via the enterprise-intel provider (ENScan: 爱企查/天眼查). Use during target_intel for red-team engagements before enriching assets. Writes candidate organizations back to the org for review. Returns a summary with counts and provider ids."
    }

    fn parameters(&self) -> Value {
        passive_intel_parameters("to discover subsidiaries for")
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

/// `recon_enrich_assets` — passive field enrichment (0.zone / quake / fofa / …):
/// collect domains / IPs / ICP / apps / emails for the organization.
pub struct ReconEnrichAssetsTool {
    pool: Arc<PgPool>,
    tools: ToolsConfigState,
}

impl ReconEnrichAssetsTool {
    pub fn new(pool: Arc<PgPool>, tools: ToolsConfigState) -> Self {
        Self { pool, tools }
    }
}

#[async_trait::async_trait]
impl Tool for ReconEnrichAssetsTool {
    fn name(&self) -> &'static str {
        "recon_enrich_assets"
    }

    fn description(&self) -> &'static str {
        "Passively enrich an organization's assets via intel providers (0.zone / quake / fofa / hunter / shodan): collect domains, IP ranges, ICP records, apps/mini-programs, and exposed emails. Use during target_intel after the engagement subject is confirmed. Writes results to the org profile + candidates. Returns a summary with counts and provider ids."
    }

    fn parameters(&self) -> Value {
        passive_intel_parameters("to enrich assets for")
    }

    async fn execute(&self, args: Value, workspace: &Path) -> Result<Value> {
        run_phase(
            &self.pool,
            &self.tools,
            &args,
            workspace,
            PassiveIntelPhase::Enrich,
            "enrich_assets",
        )
        .await
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
        "List the passive asset-intel providers (ENScan subsidiary discovery, 0.zone/quake/… enrichment) and whether each is currently usable, i.e. its credential/integration is configured. Call this FIRST during target_intel, before recon_discover_subsidiaries / recon_enrich_assets, so you only invoke providers that can actually run; for intel techniques with no available provider, record coverage as blocked (no credential) — never fabricate. Read-only: never runs a provider, never touches the target. Returns each provider's id, phase (subsidiaries/enrich), capabilities, available flag, and reason."
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
}
