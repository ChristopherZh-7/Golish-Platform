//! Provider execution runtime for Asset Intel (http / cli / lookup).
//!
//! Holds the shared provider helpers; the per-runtime runners live in the
//! `http` / `cli` / `lookup` submodules. Moved out of `mod.rs` verbatim.

use super::*;

pub(crate) mod cli;
pub(crate) mod http;
pub(crate) mod lookup;

pub(crate) use cli::*;
pub(crate) use http::*;
pub(crate) use lookup::*;

pub(crate) async fn read_vault_secret(
    pool: &sqlx::PgPool,
    tool_id: &str,
    group_id: &str,
    field_key: &str,
) -> Result<Option<String>, GolishError> {
    let name = format!("{tool_id}.{group_id}.{field_key}");
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT value FROM vault_entries \
         WHERE name = $1 \
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(&name)
    .fetch_optional(pool)
    .await?;
    if let Some((value,)) = row {
        return golish_core::vault::deobfuscate(&value)
            .map(Some)
            .map_err(|err| GolishError::Internal(format!("vault deobfuscate failed: {err}")));
    }

    if group_id == "default" {
        let legacy: Option<(String,)> = sqlx::query_as(
            "SELECT value FROM vault_entries \
             WHERE name = $1 AND entry_type = 'api_key' \
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(tool_id)
        .fetch_optional(pool)
        .await?;
        if let Some((value,)) = legacy {
            return golish_core::vault::deobfuscate(&value)
                .map(Some)
                .map_err(|err| GolishError::Internal(format!("vault deobfuscate failed: {err}")));
        }
    }

    Ok(None)
}

pub(crate) async fn resolve_http_secrets(
    pool: &sqlx::PgPool,
    asset: &golish_pentest::models::AssetIntelToolConfig,
    requests: &[golish_pentest::models::AssetIntelHttpRequest],
) -> Result<Result<HashMap<String, String>, Vec<String>>, GolishError> {
    let refs = collect_http_secret_refs(requests);
    if refs.is_empty() {
        return Ok(Ok(HashMap::new()));
    }
    let Some(requirement) = asset.requires_integration.as_ref() else {
        return Ok(Err(refs.into_iter().collect()));
    };

    let mut values = HashMap::new();
    let mut missing = Vec::new();
    for key in refs {
        let mut found = None;
        for group_id in &requirement.group_ids {
            if let Some(value) =
                read_vault_secret(pool, &requirement.tool_id, group_id, &key).await?
            {
                found = Some(value);
                break;
            }
        }
        if let Some(value) = found {
            values.insert(key, value);
        } else {
            missing.push(key);
        }
    }

    if missing.is_empty() {
        Ok(Ok(values))
    } else {
        Ok(Err(missing))
    }
}

/// Compute the `(provider_id, display_name)` pair shared by the asset-intel
/// runners: `provider_id` falls back to the tool id, and `display_name` falls
/// back to the tool name when the descriptor leaves it blank.
pub(crate) fn provider_identity(
    tool: &ToolConfig,
    asset: &golish_pentest::models::AssetIntelToolConfig,
) -> (String, String) {
    let provider_id = provider_id_for_tool(tool).unwrap_or_else(|| tool.id.clone());
    let display_name = if asset.display_name.trim().is_empty() {
        tool.name.clone()
    } else {
        asset.display_name.clone()
    };
    (provider_id, display_name)
}

/// Emit the `ProviderStarted` stream event shared by the http_json / cli_json
/// runners.
pub(crate) fn emit_provider_started(
    sink: Option<&EventEmitterHandle>,
    run_id: &str,
    provider_id: &str,
    display_name: String,
    runtime: AssetIntelProviderRuntimeKind,
) {
    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderStarted {
            run_id: run_id.to_string(),
            provider_id: provider_id.to_string(),
            display_name,
            runtime,
        },
    );
}

/// Emit the terminal `ProviderCompleted` stream event and package the runner's
/// 4-tuple result. The event's `provider_id` is taken from `status.provider_id`
/// (which every call site sets to the same value), so callers only pass the
/// status, candidate count, and payload once.
pub(crate) fn finish_provider_run(
    sink: Option<&EventEmitterHandle>,
    run_id: &str,
    status: AssetIntelProviderRunStatus,
    candidate_count: usize,
    candidates: OrganizationCandidates,
    value: Value,
    profile_entries: Vec<ProfileFieldEntry>,
) -> Result<
    (
        AssetIntelProviderRunStatus,
        OrganizationCandidates,
        Value,
        Vec<ProfileFieldEntry>,
    ),
    GolishError,
> {
    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderCompleted {
            run_id: run_id.to_string(),
            provider_id: status.provider_id.clone(),
            status: status.clone(),
            candidate_count,
        },
    );
    Ok((status, candidates, value, profile_entries))
}
