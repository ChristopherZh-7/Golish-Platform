//! Agent-facing read-only listing of passive asset-intel providers and whether
//! each is currently usable (its credential / integration is configured).
//!
//! The harness `target_intel` stage calls this (via `recon_list_providers`)
//! BEFORE `recon_discover_subsidiaries` / `recon_enrich_assets` so the AI only
//! invokes providers that can actually run, and records the rest as blocked
//! (no credential) instead of fabricating coverage (AGENTS.md I8). Reuses the
//! integrations resolver + storage backends so the configured-check is identical
//! to the GUI's "集成" page (ENScan cookie in external_file, 0.zone / quake
//! api_key in vault) — checking only the vault would wrongly mark ENScan
//! unavailable.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use sqlx::PgPool;

use golish_app_core::GolishError;
use golish_integrations::{DefaultSchemaResolver, Field, IntegrationGroup, SchemaResolver};

use crate::integrations::state::{build_integration_resolver, map_err, pick_readonly_backend};

use super::capability::{expand_provider_tools, provider_has_subsidiaries, provider_id_for_tool};
use super::ToolsConfigState;

/// One passive provider's availability snapshot, returned to the agent.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderAvailability {
    pub provider_id: String,
    pub display_name: String,
    /// `subsidiaries` (discovery phase) or `enrich` (field enrichment phase).
    pub phase: &'static str,
    pub capabilities: Vec<String>,
    /// `true` when the provider's required credential is configured and the
    /// provider can actually be invoked right now.
    pub available: bool,
    pub requires_tool_id: Option<String>,
    pub requires_group: Option<String>,
    /// Human-readable status: `configured` / `credential not configured …` / an
    /// error string when the integration schema could not be resolved.
    pub reason: String,
}

/// Pure check: are the group's required credentials all present (non-empty) in
/// the cleartext field map? Falls back to "every secret field" when the group
/// declares no `required` field. Extracted so it is unit-testable without a DB.
pub(crate) fn credentials_satisfied(
    group: &IntegrationGroup,
    cleartext: &HashMap<String, String>,
) -> bool {
    let required: Vec<&Field> = group.fields.iter().filter(|f| f.required).collect();
    let check: Vec<&Field> = if required.is_empty() {
        group
            .fields
            .iter()
            .filter(|f| f.field_type.is_secret())
            .collect()
    } else {
        required
    };
    if check.is_empty() {
        return false;
    }
    check.iter().all(|f| {
        cleartext
            .get(&f.key)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    })
}

/// List every enabled asset-intel provider with a usable / unusable verdict.
///
/// Read-only: resolves each provider's integration schema and reads its stored
/// credentials to decide `available`. It never runs the provider and never
/// touches the target.
pub async fn list_provider_availability(
    pool: Arc<PgPool>,
    tools: ToolsConfigState,
) -> Result<Vec<ProviderAvailability>, GolishError> {
    let cfg = tools.0.get().await;
    let scan = golish_pentest::scan_toolsconfig_with_status(&cfg.toolsconfig_dir, cfg.tools_dir());
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }
    let resolver = build_integration_resolver(cfg.toolsconfig_dir.clone());
    let tools_dir = cfg.tools_dir().to_path_buf();

    let mut out = Vec::new();
    for tool in expand_provider_tools(&scan.tools) {
        let Some(asset) = tool.asset_intel.as_ref() else {
            continue;
        };
        if !asset.enabled {
            continue;
        }
        let provider_id = provider_id_for_tool(&tool).unwrap_or_else(|| tool.id.clone());
        let display_name = if asset.display_name.trim().is_empty() {
            tool.name.clone()
        } else {
            asset.display_name.clone()
        };
        let phase = if provider_has_subsidiaries(&tool) {
            "subsidiaries"
        } else {
            "enrich"
        };

        let (available, reason, requires_tool_id, requires_group) =
            match asset.requires_integration.as_ref() {
                None => (true, "no credential required".to_string(), None, None),
                Some(req) => {
                    let group_id = req.group_ids.first().cloned().unwrap_or_default();
                    match provider_credential_configured(
                        &resolver,
                        &pool,
                        &tools_dir,
                        &req.tool_id,
                        &group_id,
                    )
                    .await
                    {
                        Ok(true) => (
                            true,
                            "configured".to_string(),
                            Some(req.tool_id.clone()),
                            Some(group_id),
                        ),
                        Ok(false) => (
                            false,
                            format!(
                                "credential not configured — set it in Integrations ({} / {})",
                                req.tool_id, group_id
                            ),
                            Some(req.tool_id.clone()),
                            Some(group_id),
                        ),
                        Err(e) => (
                            false,
                            format!("could not resolve integration: {e}"),
                            Some(req.tool_id.clone()),
                            Some(group_id),
                        ),
                    }
                }
            };

        out.push(ProviderAvailability {
            provider_id,
            display_name,
            phase,
            capabilities: asset.capabilities.clone(),
            available,
            requires_tool_id,
            requires_group,
            reason,
        });
    }
    Ok(out)
}

/// Resolve one provider's integration schema and read its stored credentials to
/// decide whether the required fields are configured.
async fn provider_credential_configured(
    resolver: &DefaultSchemaResolver,
    pool: &Arc<PgPool>,
    tools_dir: &Path,
    tool_id: &str,
    group_id: &str,
) -> Result<bool, GolishError> {
    let resolved = resolver.get(tool_id).await.map_err(map_err)?;
    let schema = &resolved.schema;
    let Some(group) = schema.groups.iter().find(|g| g.id == group_id) else {
        return Ok(false);
    };
    let Some(backend) =
        pick_readonly_backend(schema, (**pool).clone(), Some(tools_dir.to_path_buf()))
    else {
        return Ok(false);
    };
    let cleartext = backend
        .read_cleartext(tool_id, group_id, schema)
        .await
        .map_err(map_err)?;
    Ok(credentials_satisfied(group, &cleartext))
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_integrations::schema::{Field, FieldType};

    fn secret_field(key: &str, required: bool) -> Field {
        Field {
            key: key.to_string(),
            label: key.to_string(),
            field_type: FieldType::SecretText,
            placeholder: None,
            required,
            rows: None,
            options: Vec::new(),
            pattern: None,
        }
    }

    fn group_with(fields: Vec<Field>) -> IntegrationGroup {
        IntegrationGroup {
            id: "default".to_string(),
            name: "Default".to_string(),
            description: None,
            icon: None,
            help_url: None,
            fields,
            test: None,
            capture: None,
        }
    }

    #[test]
    fn satisfied_when_required_secret_present() {
        let group = group_with(vec![secret_field("api_key", true)]);
        let mut ct = HashMap::new();
        ct.insert("api_key".to_string(), "abc123".to_string());
        assert!(credentials_satisfied(&group, &ct));
    }

    #[test]
    fn not_satisfied_when_required_missing_or_blank() {
        let group = group_with(vec![secret_field("api_key", true)]);
        assert!(!credentials_satisfied(&group, &HashMap::new()));
        let mut blank = HashMap::new();
        blank.insert("api_key".to_string(), "   ".to_string());
        assert!(!credentials_satisfied(&group, &blank));
    }

    #[test]
    fn falls_back_to_secret_fields_when_none_required() {
        // No field marked required → check every secret field instead.
        let group = group_with(vec![secret_field("cookie", false)]);
        let mut ct = HashMap::new();
        ct.insert("cookie".to_string(), "session=1".to_string());
        assert!(credentials_satisfied(&group, &ct));
        assert!(!credentials_satisfied(&group, &HashMap::new()));
    }

    #[test]
    fn not_satisfied_when_group_has_no_checkable_field() {
        let group = group_with(Vec::new());
        assert!(!credentials_satisfied(&group, &HashMap::new()));
    }
}
