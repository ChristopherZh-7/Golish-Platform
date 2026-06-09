//! Native intel-provider runner: bridges the `golish-intel-providers`
//! registry (fofa / hunter / shodan / …) into the asset_intel pipeline so the
//! harness `target_intel` auto-enrich can use them like the http_json providers
//! (0.zone / quake). Credentials come from the same vault path as http_json
//! (`read_vault_secret`, which carries the legacy `name=tool_id` fallback), so
//! availability detection and reading stay consistent regardless of which
//! settings page configured the key.

use std::collections::HashMap;

use super::super::*;
use crate::intel_providers::provider_registry;
use crate::organizations::{OrganizationCandidate, OrganizationCandidateKind};
use golish_intel_providers::{ProviderRecord, QueryType};

/// Parse the toolsconfig `query_type` string into a provider `QueryType`.
/// Unknown / "site" → `Site` (every native provider passes raw DSL through on
/// `Site`, which is what the org-name templates rely on).
pub(crate) fn parse_query_type(s: &str) -> QueryType {
    match s {
        "domain" => QueryType::Domain,
        "cert" => QueryType::Cert,
        "asn" => QueryType::Asn,
        "cidr" => QueryType::Cidr,
        _ => QueryType::Site,
    }
}

/// Append a `ProfileFieldEntry` when `src_key` is present and non-empty.
fn push_field(
    out: &mut Vec<ProfileFieldEntry>,
    fields: &HashMap<String, String>,
    kind: golish_pentest::models::AssetIntelProfileFieldTarget,
    target_field: String,
    src_key: &str,
) {
    if let Some(value) = fields.get(src_key) {
        let value = value.trim();
        if !value.is_empty() {
            out.push(ProfileFieldEntry {
                target_kind: kind,
                target_field,
                value: value.to_string(),
            });
        }
    }
}

/// Map one [`ProviderRecord`] into a surface Target candidate (keyed on the
/// most stable identifier: domain > host > ip) plus organization-profile field
/// entries. Mirrors the `profile_fields` naming used by `0-zone.json` /
/// `quake.json` so native + http_json data land in the same columns.
pub(crate) fn bridge_record(
    provider_id: &str,
    rec: &ProviderRecord,
) -> (Option<OrganizationCandidate>, Vec<ProfileFieldEntry>) {
    use golish_pentest::models::AssetIntelProfileFieldTarget as T;
    let f = &rec.fields;
    let mut profile = Vec::new();
    push_field(&mut profile, f, T::Scalar, "domains".into(), "domain");
    push_field(&mut profile, f, T::Scalar, "ip_ranges".into(), "ip");
    push_field(&mut profile, f, T::Scalar, "certificates".into(), "cert");
    push_field(
        &mut profile,
        f,
        T::Intel,
        format!("{provider_id}_http_titles"),
        "title",
    );
    push_field(
        &mut profile,
        f,
        T::Intel,
        format!("{provider_id}_http_servers"),
        "server",
    );

    let host = f
        .get("domain")
        .or_else(|| f.get("host"))
        .or_else(|| f.get("ip"))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    let candidate = host.map(|value| OrganizationCandidate {
        id: String::new(),
        kind: OrganizationCandidateKind::Target,
        label: value.clone(),
        value,
        source: provider_id.to_string(),
        confidence: 0.7,
        status: "candidate".to_string(),
        evidence: serde_json::json!({
            "provider": provider_id,
            "query_type": rec.query_type.as_str(),
        }),
        created_at: golish_core::time::now_ms(),
    });
    (candidate, profile)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_native_provider(
    pool: &sqlx::PgPool,
    tool: &ToolConfig,
    _project_root: &Path,
    run_id: &str,
    company_name: &str,
    _config: &AssetIntelHydrateConfig,
    sink: Option<&EventEmitterHandle>,
) -> Result<
    (
        AssetIntelProviderRunStatus,
        OrganizationCandidates,
        Value,
        Vec<ProfileFieldEntry>,
    ),
    GolishError,
> {
    let Some(asset) = tool.asset_intel.as_ref() else {
        return Err(GolishError::Validation(format!(
            "tool '{}' has no asset_intel descriptor",
            tool.id
        )));
    };
    let (provider_id, display_name) = provider_identity(tool, asset);
    let golish_pentest::models::AssetIntelRuntimeConfig::NativeProvider {
        provider_id: reg_id,
        queries,
    } = &asset.runtime
    else {
        return Err(GolishError::Validation(format!(
            "tool '{}' is not a native_provider",
            tool.id
        )));
    };

    emit_provider_started(
        sink,
        run_id,
        &provider_id,
        display_name,
        AssetIntelProviderRuntimeKind::HttpJson,
    );

    // Credential: same vault path as http_json. `read_vault_secret` checks
    // `{tool_id}.{group_id}.api_key` then falls back to the legacy
    // `name=tool_id, entry_type='api_key'` row for single-field default groups.
    let (tool_id, group_id) = match asset.requires_integration.as_ref() {
        Some(req) => (
            req.tool_id.clone(),
            req.group_ids
                .first()
                .cloned()
                .unwrap_or_else(|| "default".into()),
        ),
        None => (provider_id.clone(), "default".into()),
    };
    let key = match read_vault_secret(pool, &tool_id, &group_id, "api_key").await? {
        Some(k) if !k.trim().is_empty() => k,
        _ => {
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Unavailable,
                message: format!("no api key configured for {provider_id}"),
            };
            return finish_provider_run(
                sink,
                run_id,
                status,
                0,
                OrganizationCandidates::default(),
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "unavailable",
                    "reason": "missing_secrets",
                }),
                Vec::new(),
            );
        }
    };

    let registry = provider_registry();
    let Some(provider) = registry.get(reg_id) else {
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Failed,
            message: format!("native provider '{reg_id}' not in registry"),
        };
        return finish_provider_run(
            sink,
            run_id,
            status,
            0,
            OrganizationCandidates::default(),
            serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "state": "failed",
                "reason": "unknown_provider",
            }),
            Vec::new(),
        );
    };

    let mut candidates = OrganizationCandidates::default();
    let mut profile_entries: Vec<ProfileFieldEntry> = Vec::new();
    let mut request_evidence = Vec::new();
    for q in queries {
        let rendered = q.template.replace("{{company_name}}", company_name);
        let qt = parse_query_type(&q.query_type);
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderProgress {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                message: format!("querying '{rendered}'"),
                stream: AssetIntelStreamSource::System,
            },
        );
        match provider.query(qt, &rendered, &key).await {
            Ok(records) => {
                for rec in &records {
                    let (cand, profile) = bridge_record(&provider_id, rec);
                    profile_entries.extend(profile);
                    if let Some(c) = cand {
                        candidates.targets.push(c);
                    }
                }
                request_evidence.push(serde_json::json!({
                    "query": rendered,
                    "records": records.len(),
                }));
            }
            Err(e) => {
                tracing::warn!(
                    provider = %provider_id,
                    query = %rendered,
                    error = %e,
                    "native provider query failed"
                );
                request_evidence.push(serde_json::json!({
                    "query": rendered,
                    "error": e.to_string(),
                }));
            }
        }
    }

    let candidate_count = candidates.organizations.len() + candidates.targets.len();
    let record_count = candidate_count + profile_entries.len();
    let state = if record_count == 0 {
        AssetIntelProviderRunState::CheckedEmpty
    } else {
        AssetIntelProviderRunState::Completed
    };
    tracing::info!(
        provider = %provider_id,
        run_id,
        candidate_count,
        profile_field_count = profile_entries.len(),
        "asset_intel native provider completed"
    );
    let status = AssetIntelProviderRunStatus {
        provider_id: provider_id.clone(),
        status: state,
        message: format!("{provider_id} produced {record_count} record(s)"),
    };
    finish_provider_run(
        sink,
        run_id,
        status,
        candidate_count,
        candidates,
        serde_json::json!({
            "provider": provider_id,
            "runId": run_id,
            "state": if record_count == 0 { "checked_empty" } else { "completed" },
            "candidateCount": candidate_count,
            "profileFieldCount": profile_entries.len(),
            "queries": request_evidence,
        }),
        profile_entries,
    )
}
