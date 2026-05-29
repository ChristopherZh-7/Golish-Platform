//! Asset Intel service for Discover Assets engagements.
//!
//! Phase 1 keeps this layer provider-agnostic: the workspace asks for
//! candidates, providers return normalized records, and only approved
//! candidates become scope in later phases.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

use golish_core::{emit_opt, EventEmitterHandle};
use golish_pentest::models::ToolConfig;

use crate::error::GolishError;
use crate::event_emitter::TauriEventEmitter;
use crate::state::DbState;
use crate::tools::organizations::{upsert_organization_candidates_for_org, OrganizationCandidates};
#[cfg(test)]
use crate::tools::organizations::{OrganizationCandidate, OrganizationCandidateKind};
use crate::tools::pentest::PentestState;

mod asn;
mod capability;
mod merge;
mod normalize;
mod profile_patch;
mod promote;
mod records;
mod template;
mod types;
pub(crate) use asn::{
    collect_public_ips_for_asn_lookup, normalize_asn, parse_team_cymru_asn_response,
    profile_asn_entries_from_mappings, IpAsnMapping, TEAM_CYMRU_ASN_LOOKUP_TIMEOUT_SECS,
    TEAM_CYMRU_WHOIS_ADDR,
};
#[cfg(test)]
pub(crate) use capability::{expand_provider_tools, provider_has_subsidiaries};
pub(crate) use capability::{
    provider_descriptors_from_tools, provider_id_for_tool, provider_output_is_trusted,
    select_asset_intel_providers, select_enrichment_providers, select_subsidiary_providers,
};
pub(crate) use merge::{flatten_candidates, merge_candidates};
pub(crate) use normalize::{
    extract_profile_field_entries, filter_passes, resolve_field_ref, select_json_values,
};
pub(crate) use profile_patch::{
    build_profile_patch_from_entries, merge_profile_patch_with_existing,
};
#[cfg(test)]
pub(crate) use promote::AutoPromoteSkipReason;
pub(crate) use promote::{auto_promote_child_decisions, clear_engagement_candidates_from_intel};
#[cfg(test)]
pub(crate) use records::normalize_provider_records;
pub(crate) use records::{
    dedupe_lookup_matches, extract_lookup_matches, normalize_json_document,
    normalize_json_with_descriptor,
};
pub(crate) use template::{
    collect_http_secret_refs, render_asset_intel_skill_args, render_http_json_value,
    render_http_template, render_lookup_skill_args, split_command_args,
};
#[cfg(test)]
pub(crate) use types::enrichment_hydrate_config_for_organization;
pub(crate) use types::{discovery_hydrate_config, enrichment_hydrate_config};
pub use types::{
    AssetIntelBatchSource, AssetIntelCapability, AssetIntelEnrichBatchArgs,
    AssetIntelEnrichBatchResult, AssetIntelEnrichBatchSkip, AssetIntelEnrichOrganizationArgs,
    AssetIntelHydrateArgs, AssetIntelHydrateConfig, AssetIntelIntegrationRequirement,
    AssetIntelLookupRequest, AssetIntelLookupResult, AssetIntelProviderDescriptor,
    AssetIntelProviderRecord, AssetIntelProviderRunState, AssetIntelProviderRunStatus,
    AssetIntelProviderRuntimeKind, AssetIntelProviderStatus, AssetIntelRun, AssetIntelRunStatus,
    AssetIntelStreamEvent, AssetIntelStreamSource, LookupCompanyMatch, ProfileFieldEntry,
};

/// Tauri event name used for all Asset Intel streaming events.
///
/// The frontend listens once on this channel and filters payloads by `runId`.
/// Kept as a constant so backend + frontend share a single source of truth
/// (frontend re-imports the literal in `lib/api/asset-intel.ts`).
pub const ASSET_INTEL_EVENT: &str = "asset-intel:event";

fn emit_event(sink: Option<&EventEmitterHandle>, event: AssetIntelStreamEvent) {
    emit_opt(sink, ASSET_INTEL_EVENT, &event);
}

async fn clear_engagement_candidates_for_org(
    pool: &sqlx::PgPool,
    organization_id: Uuid,
) -> Result<(), GolishError> {
    let Some(row) = golish_db::repo::organizations::get_one(pool, organization_id).await? else {
        return Err(GolishError::NotFound(format!(
            "organization {organization_id}"
        )));
    };
    let intel = clear_engagement_candidates_from_intel(row.intel)?;
    let patch = golish_db::repo::organizations::ProfilePatch {
        intel: Some(intel),
        ..Default::default()
    };
    golish_db::repo::organizations::update_profile(pool, organization_id, &patch)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {organization_id}")))?;
    Ok(())
}

async fn auto_promote_discovered_children(
    pool: &sqlx::PgPool,
    parent: &golish_db::models::Organization,
    candidates: &OrganizationCandidates,
    policy: &golish_pentest::models::AssetIntelDiscoveryConfig,
) -> Result<Value, GolishError> {
    let existing = golish_db::repo::organizations::list(pool, &parent.project_path).await?;
    let existing_child_names: HashSet<String> = existing
        .iter()
        .filter(|org| org.parent_id == Some(parent.id))
        .map(|org| org.name.trim().to_lowercase())
        .collect();
    let decisions = auto_promote_child_decisions(candidates, policy, &existing_child_names);

    let mut created = Vec::new();
    let mut skipped = Vec::new();
    for decision in decisions {
        let name = decision.candidate.value.trim();
        if decision.promote {
            let child = golish_db::repo::organizations::create(
                pool,
                &parent.project_path,
                name,
                Some(parent.id),
                &format!(
                    "Auto-promoted from {} investment discovery",
                    decision.candidate.source
                ),
                "",
            )
            .await?;
            let mut intel = serde_json::Map::new();
            intel.insert(
                "asset_intel_discovery".into(),
                serde_json::json!({
                    "parentOrganizationId": parent.id.to_string(),
                    "source": decision.candidate.source,
                    "ownershipPercent": decision.ownership_percent,
                    "evidence": decision.candidate.evidence,
                }),
            );
            let patch = golish_db::repo::organizations::ProfilePatch {
                intel: Some(Value::Object(intel)),
                ..Default::default()
            };
            golish_db::repo::organizations::update_profile(pool, child.id, &patch).await?;
            created.push(serde_json::json!({
                "organizationId": child.id.to_string(),
                "name": child.name,
                "ownershipPercent": decision.ownership_percent,
                "source": decision.candidate.source,
            }));
        } else {
            skipped.push(serde_json::json!({
                "name": name,
                "source": decision.candidate.source,
                "ownershipPercent": decision.ownership_percent,
                "reason": decision.reason,
            }));
        }
    }
    clear_engagement_candidates_for_org(pool, parent.id).await?;

    Ok(serde_json::json!({
        "kind": "auto_promote_children",
        "policy": policy,
        "clearedCandidates": true,
        "created": created,
        "skipped": skipped,
    }))
}

async fn lookup_team_cymru_asns(ips: &[IpAddr]) -> Result<Vec<IpAsnMapping>, String> {
    if ips.is_empty() {
        return Ok(Vec::new());
    }
    let timeout = Duration::from_secs(TEAM_CYMRU_ASN_LOOKUP_TIMEOUT_SECS);
    let mut stream = tokio::time::timeout(
        timeout,
        tokio::net::TcpStream::connect(TEAM_CYMRU_WHOIS_ADDR),
    )
    .await
    .map_err(|_| "timed out connecting to Team Cymru whois".to_string())?
    .map_err(|err| format!("connect failed: {err}"))?;
    let query = format!(
        "begin\nverbose\n{}\nend\n",
        ips.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
    tokio::time::timeout(timeout, stream.write_all(query.as_bytes()))
        .await
        .map_err(|_| "timed out writing Team Cymru query".to_string())?
        .map_err(|err| format!("write failed: {err}"))?;
    let mut response = String::new();
    tokio::time::timeout(timeout, stream.read_to_string(&mut response))
        .await
        .map_err(|_| "timed out reading Team Cymru response".to_string())?
        .map_err(|err| format!("read failed: {err}"))?;
    Ok(parse_team_cymru_asn_response(&response))
}

async fn enrich_0zone_asns_from_ip_ranges(
    provider_id: &str,
    run_id: &str,
    profile_entries: &mut Vec<ProfileFieldEntry>,
    sink: Option<&EventEmitterHandle>,
) -> Option<Value> {
    if provider_id != "0.zone"
        || profile_entries
            .iter()
            .any(|entry| entry.target_field == "asns" && !entry.value.trim().is_empty())
    {
        return None;
    }
    let ips = collect_public_ips_for_asn_lookup(profile_entries);
    if ips.is_empty() {
        return None;
    }
    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderProgress {
            run_id: run_id.to_string(),
            provider_id: provider_id.to_string(),
            message: format!("deriving ASN from {} public IP(s)", ips.len()),
            stream: AssetIntelStreamSource::System,
        },
    );
    match lookup_team_cymru_asns(&ips).await {
        Ok(mappings) => {
            let derived = profile_asn_entries_from_mappings(&mappings);
            let asn_count = derived.len();
            profile_entries.extend(derived);
            Some(serde_json::json!({
                "requestId": "team-cymru-ip-to-asn",
                "state": if asn_count == 0 { "checked_empty" } else { "completed" },
                "queriedIpCount": ips.len(),
                "asnCount": asn_count,
            }))
        }
        Err(error) => {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                error,
                "asset_intel derived ASN lookup failed"
            );
            Some(serde_json::json!({
                "requestId": "team-cymru-ip-to-asn",
                "state": "failed",
                "queriedIpCount": ips.len(),
                "error": error,
            }))
        }
    }
}

async fn read_vault_secret(
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

async fn resolve_http_secrets(
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

async fn run_http_json_provider(
    pool: &sqlx::PgPool,
    tool: &ToolConfig,
    run_id: &str,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
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
    let provider_id = provider_id_for_tool(tool).unwrap_or_else(|| tool.id.clone());
    let display_name = if asset.display_name.trim().is_empty() {
        tool.name.clone()
    } else {
        asset.display_name.clone()
    };
    let golish_pentest::models::AssetIntelRuntimeConfig::HttpJson { requests } = &asset.runtime
    else {
        return Err(GolishError::Validation(format!(
            "tool '{}' is not an http_json provider",
            tool.id
        )));
    };

    // Sequential per-request loop; accumulator stays here so every early
    // return path can hand whatever has already been hydrated up to the
    // hydrate orchestrator (we don't want to drop master-record fields
    // because a later request 500'd).
    let mut profile_entries: Vec<ProfileFieldEntry> = Vec::new();

    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderStarted {
            run_id: run_id.to_string(),
            provider_id: provider_id.clone(),
            display_name,
            runtime: AssetIntelProviderRuntimeKind::HttpJson,
        },
    );

    if requests.is_empty() {
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Unavailable,
            message: "http_json provider has no requests".into(),
        };
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderCompleted {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                status: status.clone(),
                candidate_count: 0,
            },
        );
        return Ok((
            status,
            OrganizationCandidates::default(),
            serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "state": "unavailable",
                "reason": "no_requests",
            }),
            profile_entries,
        ));
    }

    let secrets = match resolve_http_secrets(pool, asset, requests).await? {
        Ok(values) => values,
        Err(missing) => {
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Unavailable,
                message: format!("missing integration secret(s): {}", missing.join(", ")),
            };
            emit_event(
                sink,
                AssetIntelStreamEvent::ProviderCompleted {
                    run_id: run_id.to_string(),
                    provider_id: provider_id.clone(),
                    status: status.clone(),
                    candidate_count: 0,
                },
            );
            return Ok((
                status,
                OrganizationCandidates::default(),
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "unavailable",
                    "reason": "missing_secrets",
                    "missing": missing,
                }),
                profile_entries,
            ));
        }
    };

    let client = reqwest::Client::builder()
        .user_agent(concat!("golish/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| GolishError::Internal(format!("http client build failed: {err}")))?;
    let mut candidates = OrganizationCandidates::default();
    let mut request_evidence = Vec::new();
    for request in requests {
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderProgress {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                message: format!("requesting '{}' ({})", request.id, request.method),
                stream: AssetIntelStreamSource::System,
            },
        );
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|err| GolishError::Validation(format!("bad HTTP method: {err}")))?;
        let url = render_http_template(&request.url, company_name, config, &secrets);
        let timeout_secs = request.timeout_secs.clamp(1, 120);
        tracing::info!(
            provider = %provider_id,
            run_id,
            request_id = %request.id,
            timeout_secs,
            "running asset_intel http_json request"
        );
        let mut builder = client
            .request(method, &url)
            .timeout(Duration::from_secs(timeout_secs));
        for (name, value) in &request.headers {
            builder = builder.header(
                name,
                render_http_template(value, company_name, config, &secrets),
            );
        }
        if !request.form.is_empty() {
            let form: HashMap<String, String> = request
                .form
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        render_http_template(value, company_name, config, &secrets),
                    )
                })
                .collect();
            let mut encoded = url::form_urlencoded::Serializer::new(String::new());
            for (key, value) in &form {
                encoded.append_pair(key, value);
            }
            builder = builder
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(encoded.finish());
        } else if !request.json.is_null() {
            builder = builder.json(&render_http_json_value(
                &request.json,
                company_name,
                config,
                &secrets,
            ));
        }

        let response = match builder.send().await {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(
                    provider = %provider_id,
                    run_id,
                    request_id = %request.id,
                    error = %err,
                    "asset_intel http_json request failed"
                );
                let status = AssetIntelProviderRunStatus {
                    provider_id: provider_id.clone(),
                    status: AssetIntelProviderRunState::Failed,
                    message: format!("request '{}' failed: {err}", request.id),
                };
                let count = candidates.organizations.len() + candidates.targets.len();
                emit_event(
                    sink,
                    AssetIntelStreamEvent::ProviderCompleted {
                        run_id: run_id.to_string(),
                        provider_id: provider_id.clone(),
                        status: status.clone(),
                        candidate_count: count,
                    },
                );
                return Ok((
                    status,
                    candidates,
                    serde_json::json!({
                        "provider": provider_id,
                        "runId": run_id,
                        "state": "failed",
                        "reason": "request_failed",
                        "requestId": request.id,
                        "error": err.to_string(),
                        "candidateCount": count,
                    }),
                    profile_entries,
                ));
            }
        };
        let http_status = response.status();
        let body = response.text().await.unwrap_or_default();
        let preview: String = body.chars().take(512).collect();
        if !http_status.is_success() {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                request_id = %request.id,
                status = http_status.as_u16(),
                "asset_intel http_json request returned non-success status"
            );
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!("request '{}' returned HTTP {http_status}", request.id),
            };
            let count = candidates.organizations.len() + candidates.targets.len();
            emit_event(
                sink,
                AssetIntelStreamEvent::ProviderCompleted {
                    run_id: run_id.to_string(),
                    provider_id: provider_id.clone(),
                    status: status.clone(),
                    candidate_count: count,
                },
            );
            return Ok((
                status,
                candidates,
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "failed",
                    "reason": "http_status",
                    "requestId": request.id,
                    "status": http_status.as_u16(),
                    "preview": preview,
                    "candidateCount": count,
                }),
                profile_entries,
            ));
        }

        if let Some((next, profile)) =
            normalize_json_document(&provider_id, run_id, &asset.normalize, &body)
        {
            profile_entries.extend(profile);
            let added_total = next.organizations.len() + next.targets.len();
            if added_total > 0 {
                let mut delta = OrganizationCandidates::default();
                for item in next.organizations.iter() {
                    delta.organizations.push(item.clone());
                }
                for item in next.targets.iter() {
                    delta.targets.push(item.clone());
                }
                merge_candidates(&mut candidates, next);
                emit_event(
                    sink,
                    AssetIntelStreamEvent::ProviderBatch {
                        run_id: run_id.to_string(),
                        provider_id: provider_id.clone(),
                        candidates: delta,
                        source: AssetIntelBatchSource::Http,
                        artifact: None,
                        request_id: Some(request.id.clone()),
                    },
                );
            }
        }
        request_evidence.push(serde_json::json!({
            "requestId": request.id,
            "status": http_status.as_u16(),
        }));
    }

    if let Some(evidence) =
        enrich_0zone_asns_from_ip_ranges(&provider_id, run_id, &mut profile_entries, sink).await
    {
        request_evidence.push(evidence);
    }

    let total = candidates.organizations.len() + candidates.targets.len();
    let state = if total == 0 {
        AssetIntelProviderRunState::CheckedEmpty
    } else {
        AssetIntelProviderRunState::Completed
    };
    tracing::info!(
        provider = %provider_id,
        run_id,
        candidate_count = total,
        state = ?state,
        "asset_intel http_json provider completed"
    );
    let status = AssetIntelProviderRunStatus {
        provider_id: provider_id.clone(),
        status: state,
        message: if total == 0 {
            format!("{provider_id} completed with no candidates")
        } else {
            format!("{provider_id} normalized {total} candidate(s)")
        },
    };
    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderCompleted {
            run_id: run_id.to_string(),
            provider_id: provider_id.clone(),
            status: status.clone(),
            candidate_count: total,
        },
    );
    Ok((
        status,
        candidates,
        serde_json::json!({
            "provider": provider_id,
            "runId": run_id,
            "state": if total == 0 { "checked_empty" } else { "completed" },
            "candidateCount": total,
            "requests": request_evidence,
        }),
        profile_entries,
    ))
}

fn asset_intel_provider_output_dir(
    project_root: &Path,
    run_id: &str,
    provider_id: &str,
) -> PathBuf {
    golish_projects::file_storage::tool_output_dir(project_root, "asset-intel")
        .join(run_id)
        .join(provider_id)
}

/// Max characters of any single stdout/stderr line forwarded to the frontend.
///
/// Long PTY/OSC dumps (terminal control sequences) can balloon individual
/// lines into multi-kilobyte chunks; truncating here keeps the event stream
/// useful and bounds memory cost per emit.
const PROVIDER_PROGRESS_LINE_LIMIT: usize = 512;

/// Polling interval for the `out_dir` artifact watcher (cli_json runtime).
///
/// The frontend's perceived "first candidate in N seconds" is bounded by
/// this interval. Tuned to a sweet spot: small enough to feel live (<1s),
/// large enough to avoid hot-looping `read_dir` during long scans.
const ARTIFACT_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Shared, normalize-and-emit-once accumulator used by the cli_json runner.
///
/// Keeping the accumulator + the cancel flag in a single Arc-wrapped struct
/// lets us hand a cheap clone to every background task (stdout reader,
/// stderr reader, artifact watcher) without juggling individual Arcs.
#[derive(Debug)]
struct CliJsonStreamShared {
    candidates: TokioMutex<OrganizationCandidates>,
    /// Profile field entries lifted out of the same raw JSON documents.
    /// Stored separately from candidates because they target the master
    /// record (credit_code / industry / contacts / intel keys), not the
    /// review queue. The hydrate top-level merges these into a single
    /// `OrganizationProfilePatch` after the provider finishes.
    profile_entries: TokioMutex<Vec<ProfileFieldEntry>>,
    progress_buffer: TokioMutex<String>,
    cancel: AtomicBool,
}

impl CliJsonStreamShared {
    fn new() -> Self {
        Self {
            candidates: TokioMutex::new(OrganizationCandidates::default()),
            profile_entries: TokioMutex::new(Vec::new()),
            progress_buffer: TokioMutex::new(String::new()),
            cancel: AtomicBool::new(false),
        }
    }
}

fn truncate_progress_line(raw: &str) -> String {
    let cleaned = raw.trim_end_matches(['\r', '\n']).trim();
    if cleaned.chars().count() <= PROVIDER_PROGRESS_LINE_LIMIT {
        cleaned.to_string()
    } else {
        let mut out: String = cleaned.chars().take(PROVIDER_PROGRESS_LINE_LIMIT).collect();
        out.push_str(" … (truncated)");
        out
    }
}

/// Try to normalize a single stdout line as JSON; emit a Batch if it yields
/// candidates. Non-JSON or empty-result lines are returned to the caller so
/// they can be emitted as Progress instead.
async fn handle_stdout_line(
    line: &str,
    provider_id: &str,
    run_id: &str,
    normalize: &golish_pentest::models::AssetIntelNormalizeConfig,
    shared: &CliJsonStreamShared,
    sink: Option<&EventEmitterHandle>,
) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return false;
    };
    let (next, profile) =
        normalize_json_with_descriptor(provider_id, run_id, now_millis(), normalize, &value);
    if !profile.is_empty() {
        shared.profile_entries.lock().await.extend(profile);
    }
    let added_total = next.organizations.len() + next.targets.len();
    if added_total == 0 {
        return false;
    }
    let mut guard = shared.candidates.lock().await;
    let mut delta = OrganizationCandidates::default();
    for item in next.organizations.iter() {
        delta.organizations.push(item.clone());
    }
    for item in next.targets.iter() {
        delta.targets.push(item.clone());
    }
    merge_candidates(&mut guard, next);
    drop(guard);
    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderBatch {
            run_id: run_id.to_string(),
            provider_id: provider_id.to_string(),
            candidates: delta,
            source: AssetIntelBatchSource::Stdout,
            artifact: None,
            request_id: None,
        },
    );
    true
}

/// Scan `out_dir` for JSON artifacts that have not been emitted yet; for any
/// newly-seen file, normalize its contents and emit a Batch with source =
/// artifact. Mutates `seen` so repeated calls are idempotent.
async fn scan_new_artifacts(
    out_dir: &Path,
    provider_id: &str,
    run_id: &str,
    normalize: &golish_pentest::models::AssetIntelNormalizeConfig,
    seen: &mut HashSet<PathBuf>,
    shared: &CliJsonStreamShared,
    sink: Option<&EventEmitterHandle>,
) -> Result<(), GolishError> {
    let mut files = Vec::new();
    collect_json_files(out_dir, &mut files)?;
    files.sort();
    for path in files {
        if !seen.insert(path.clone()) {
            continue;
        }
        let raw = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                tracing::debug!(
                    provider = %provider_id,
                    run_id,
                    artifact = %path.display(),
                    error = %err,
                    "asset_intel cli_json artifact read failed (skipping)"
                );
                continue;
            }
        };
        let Some((next, profile)) = normalize_json_document(provider_id, run_id, normalize, &raw)
        else {
            continue;
        };
        if !profile.is_empty() {
            shared.profile_entries.lock().await.extend(profile);
        }
        let added_total = next.organizations.len() + next.targets.len();
        if added_total == 0 {
            continue;
        }
        let mut delta = OrganizationCandidates::default();
        for item in next.organizations.iter() {
            delta.organizations.push(item.clone());
        }
        for item in next.targets.iter() {
            delta.targets.push(item.clone());
        }
        let mut guard = shared.candidates.lock().await;
        merge_candidates(&mut guard, next);
        drop(guard);
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderBatch {
                run_id: run_id.to_string(),
                provider_id: provider_id.to_string(),
                candidates: delta,
                source: AssetIntelBatchSource::Artifact,
                artifact: Some(path.display().to_string()),
                request_id: None,
            },
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_cli_json_provider(
    tool: &ToolConfig,
    tools: &[ToolConfig],
    tools_dir: &Path,
    project_root: &Path,
    run_id: &str,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
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
    let provider_id = provider_id_for_tool(tool).unwrap_or_else(|| tool.id.clone());
    let display_name = if asset.display_name.trim().is_empty() {
        tool.name.clone()
    } else {
        asset.display_name.clone()
    };
    let golish_pentest::models::AssetIntelRuntimeConfig::CliJson {
        skill_id,
        timeout_secs,
        artifact_globs: _,
        arg_bindings,
    } = &asset.runtime
    else {
        return Err(GolishError::Validation(format!(
            "tool '{}' is not a cli_json provider",
            tool.id
        )));
    };

    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderStarted {
            run_id: run_id.to_string(),
            provider_id: provider_id.clone(),
            display_name: display_name.clone(),
            runtime: AssetIntelProviderRuntimeKind::CliJson,
        },
    );

    let Some(exec) = golish_pentest::resolve_tool_executable(&tool.id, tools, tools_dir) else {
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Unavailable,
            message: format!("tool '{}' executable is unavailable", tool.id),
        };
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderCompleted {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                status: status.clone(),
                candidate_count: 0,
            },
        );
        return Ok((
            status,
            OrganizationCandidates::default(),
            serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "state": "unavailable",
                "reason": "tool_executable_unavailable",
            }),
            Vec::new(),
        ));
    };
    let Some(skill) = tool.skills.iter().find(|skill| skill.id == *skill_id) else {
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Unavailable,
            message: format!("asset intel skill '{skill_id}' is not declared"),
        };
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderCompleted {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                status: status.clone(),
                candidate_count: 0,
            },
        );
        return Ok((
            status,
            OrganizationCandidates::default(),
            serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "state": "unavailable",
                "reason": "skill_not_found",
                "skillId": skill_id,
            }),
            Vec::new(),
        ));
    };

    let out_dir = asset_intel_provider_output_dir(project_root, run_id, &provider_id);
    fs::create_dir_all(&out_dir)?;
    let rendered_args =
        render_asset_intel_skill_args(&skill.args, company_name, &out_dir, config, arg_bindings);
    let args = split_command_args(&rendered_args);
    let mut command = Command::new(&exec);
    command.args(&args);
    command.current_dir(&out_dir);
    command.kill_on_drop(true);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let timeout = Duration::from_secs((*timeout_secs).clamp(1, 900));
    tracing::info!(
        provider = %provider_id,
        run_id,
        timeout_secs = timeout.as_secs(),
        out_dir = %out_dir.display(),
        "running asset_intel cli_json provider"
    );

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                error = %err,
                "asset_intel cli_json provider failed to spawn"
            );
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Unavailable,
                message: format!("spawn failed: {err}"),
            };
            emit_event(
                sink,
                AssetIntelStreamEvent::ProviderCompleted {
                    run_id: run_id.to_string(),
                    provider_id: provider_id.clone(),
                    status: status.clone(),
                    candidate_count: 0,
                },
            );
            return Ok((
                status,
                OrganizationCandidates::default(),
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "unavailable",
                    "reason": "spawn_failed",
                    "error": err.to_string(),
                }),
                Vec::new(),
            ));
        }
    };

    let shared = Arc::new(CliJsonStreamShared::new());
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let normalize = asset.normalize.clone();

    let stdout_handle = stdout.map(|stream| {
        let shared = shared.clone();
        let sink = sink.cloned();
        let normalize = normalize.clone();
        let provider_id = provider_id.clone();
        let run_id = run_id.to_string();
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                {
                    let mut buf = shared.progress_buffer.lock().await;
                    buf.push_str(&line);
                    buf.push('\n');
                }
                let emitted = handle_stdout_line(
                    &line,
                    &provider_id,
                    &run_id,
                    &normalize,
                    &shared,
                    sink.as_ref(),
                )
                .await;
                if !emitted {
                    let msg = truncate_progress_line(&line);
                    if !msg.is_empty() {
                        emit_event(
                            sink.as_ref(),
                            AssetIntelStreamEvent::ProviderProgress {
                                run_id: run_id.clone(),
                                provider_id: provider_id.clone(),
                                message: msg,
                                stream: AssetIntelStreamSource::Stdout,
                            },
                        );
                    }
                }
            }
        })
    });

    let stderr_handle = stderr.map(|stream| {
        let shared = shared.clone();
        let sink = sink.cloned();
        let provider_id = provider_id.clone();
        let run_id = run_id.to_string();
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                {
                    let mut buf = shared.progress_buffer.lock().await;
                    buf.push_str(&line);
                    buf.push('\n');
                }
                let msg = truncate_progress_line(&line);
                if msg.is_empty() {
                    continue;
                }
                emit_event(
                    sink.as_ref(),
                    AssetIntelStreamEvent::ProviderProgress {
                        run_id: run_id.clone(),
                        provider_id: provider_id.clone(),
                        message: msg,
                        stream: AssetIntelStreamSource::Stderr,
                    },
                );
            }
        })
    });

    let watcher_handle = {
        let shared = shared.clone();
        let sink = sink.cloned();
        let normalize = normalize.clone();
        let provider_id = provider_id.clone();
        let run_id = run_id.to_string();
        let out_dir = out_dir.clone();
        tokio::spawn(async move {
            let mut seen: HashSet<PathBuf> = HashSet::new();
            while !shared.cancel.load(Ordering::Acquire) {
                if let Err(err) = scan_new_artifacts(
                    &out_dir,
                    &provider_id,
                    &run_id,
                    &normalize,
                    &mut seen,
                    &shared,
                    sink.as_ref(),
                )
                .await
                {
                    tracing::debug!(
                        provider = %provider_id,
                        run_id,
                        error = %err,
                        "asset_intel cli_json artifact watcher scan failed"
                    );
                }
                tokio::time::sleep(ARTIFACT_POLL_INTERVAL).await;
            }
        })
    };

    let exit_result = tokio::time::timeout(timeout, child.wait()).await;
    shared.cancel.store(true, Ordering::Release);
    if let Some(handle) = stdout_handle {
        let _ = handle.await;
    }
    if let Some(handle) = stderr_handle {
        let _ = handle.await;
    }
    let _ = watcher_handle.await;

    let exit_status = match exit_result {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                error = %err,
                "asset_intel cli_json provider wait failed"
            );
            let _ = child.kill().await;
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!("wait failed: {err}"),
            };
            emit_event(
                sink,
                AssetIntelStreamEvent::ProviderCompleted {
                    run_id: run_id.to_string(),
                    provider_id: provider_id.clone(),
                    status: status.clone(),
                    candidate_count: 0,
                },
            );
            let profile_entries = std::mem::take(&mut *shared.profile_entries.lock().await);
            return Ok((
                status,
                OrganizationCandidates::default(),
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "failed",
                    "reason": "wait_failed",
                    "error": err.to_string(),
                }),
                profile_entries,
            ));
        }
        Err(_) => {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                timeout_secs = timeout.as_secs(),
                "asset_intel cli_json provider timed out"
            );
            let _ = child.kill().await;
            let candidates = std::mem::take(&mut *shared.candidates.lock().await);
            let profile_entries = std::mem::take(&mut *shared.profile_entries.lock().await);
            let candidate_count = candidates.organizations.len() + candidates.targets.len();
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!("command timed out after {}s", timeout.as_secs()),
            };
            emit_event(
                sink,
                AssetIntelStreamEvent::ProviderCompleted {
                    run_id: run_id.to_string(),
                    provider_id: provider_id.clone(),
                    status: status.clone(),
                    candidate_count,
                },
            );
            return Ok((
                status,
                candidates,
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "failed",
                    "reason": "timeout",
                    "timeoutSecs": timeout.as_secs(),
                    "candidateCount": candidate_count,
                }),
                profile_entries,
            ));
        }
    };

    let mut final_seen: HashSet<PathBuf> = HashSet::new();
    if let Err(err) = scan_new_artifacts(
        &out_dir,
        &provider_id,
        run_id,
        &normalize,
        &mut final_seen,
        shared.as_ref(),
        sink,
    )
    .await
    {
        tracing::debug!(
            provider = %provider_id,
            run_id,
            error = %err,
            "asset_intel cli_json final artifact scan failed"
        );
    }

    let candidates = std::mem::take(&mut *shared.candidates.lock().await);
    let profile_entries = std::mem::take(&mut *shared.profile_entries.lock().await);
    let progress_buffer = std::mem::take(&mut *shared.progress_buffer.lock().await);
    let preview: String = progress_buffer.chars().take(512).collect();

    if !exit_status.success() {
        tracing::warn!(
            provider = %provider_id,
            run_id,
            exit_code = exit_status.code(),
            "asset_intel cli_json provider exited unsuccessfully"
        );
        let candidate_count = candidates.organizations.len() + candidates.targets.len();
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Failed,
            message: format!("command failed: {preview}"),
        };
        emit_event(
            sink,
            AssetIntelStreamEvent::ProviderCompleted {
                run_id: run_id.to_string(),
                provider_id: provider_id.clone(),
                status: status.clone(),
                candidate_count,
            },
        );
        return Ok((
            status,
            candidates,
            serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "state": "failed",
                "reason": "command_failed",
                "exitCode": exit_status.code(),
                "preview": preview,
                "candidateCount": candidate_count,
            }),
            profile_entries,
        ));
    }

    let total = candidates.organizations.len() + candidates.targets.len();
    let state = if total == 0 {
        AssetIntelProviderRunState::CheckedEmpty
    } else {
        AssetIntelProviderRunState::Completed
    };
    tracing::info!(
        provider = %provider_id,
        run_id,
        candidate_count = total,
        state = ?state,
        "asset_intel cli_json provider completed"
    );
    let status = AssetIntelProviderRunStatus {
        provider_id: provider_id.clone(),
        status: state,
        message: if total == 0 {
            format!("{provider_id} completed with no candidates")
        } else {
            format!("{provider_id} normalized {total} candidate(s)")
        },
    };
    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderCompleted {
            run_id: run_id.to_string(),
            provider_id: provider_id.clone(),
            status: status.clone(),
            candidate_count: total,
        },
    );
    Ok((
        status,
        candidates,
        serde_json::json!({
            "provider": provider_id,
            "runId": run_id,
            "state": if total == 0 { "checked_empty" } else { "completed" },
            "candidateCount": total,
            "outDir": out_dir,
        }),
        profile_entries,
    ))
}

fn collect_json_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), GolishError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            files.push(path);
        }
    }
    Ok(())
}

fn now_millis() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

/// Per-provider output directory used by the lookup runtime, scoped under
/// `<project>/.golish/tool-output/asset-intel-lookup/<runId>/<providerId>`.
/// Keeps lookup artifacts separate from full hydrate runs so cleanup is
/// trivial and there's no risk of mixing canonical vs. discovery output.
fn lookup_provider_output_dir(project_root: &Path, run_id: &str, provider_id: &str) -> PathBuf {
    golish_projects::file_storage::tool_output_dir(project_root, "asset-intel-lookup")
        .join(run_id)
        .join(provider_id)
}

/// Run a tool's `asset_intel.lookup` skill in synchronous "wait once, parse,
/// return matches" mode. Used by `asset_intel_lookup_company` to give the
/// UI a fast disambiguation list before a real hydrate.
///
/// Differences vs. `run_cli_json_provider`:
/// - No streaming events (`provider_started` / `provider_progress` / batch).
///   UI shows a single spinner, then a candidate list.
/// - No candidate / profile_fields output; only `LookupCompanyMatch` rows.
/// - Hard timeout from the descriptor, clamped to `[1, 300]` seconds.
async fn run_lookup_cli_provider(
    tool: &ToolConfig,
    tools: &[ToolConfig],
    tools_dir: &Path,
    project_root: &Path,
    run_id: &str,
    keyword: &str,
) -> Result<(AssetIntelProviderRunStatus, Vec<LookupCompanyMatch>), GolishError> {
    let asset = tool.asset_intel.as_ref().ok_or_else(|| {
        GolishError::Validation(format!("tool '{}' has no asset_intel descriptor", tool.id))
    })?;
    let provider_id = provider_id_for_tool(tool).unwrap_or_else(|| tool.id.clone());
    let lookup = match asset.lookup.as_ref() {
        Some(l) if l.enabled => l,
        _ => {
            return Ok((
                AssetIntelProviderRunStatus {
                    provider_id: provider_id.clone(),
                    status: AssetIntelProviderRunState::Unavailable,
                    message: format!("'{provider_id}' does not declare a lookup runtime"),
                },
                Vec::new(),
            ));
        }
    };

    if !matches!(
        asset.runtime,
        golish_pentest::models::AssetIntelRuntimeConfig::CliJson { .. }
    ) {
        return Ok((
            AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Unavailable,
                message: "lookup is only supported for cli_json providers in this release".into(),
            },
            Vec::new(),
        ));
    }

    let Some(exec) = golish_pentest::resolve_tool_executable(&tool.id, tools, tools_dir) else {
        return Ok((
            AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Unavailable,
                message: format!("tool '{}' executable is unavailable", tool.id),
            },
            Vec::new(),
        ));
    };
    let Some(skill) = tool.skills.iter().find(|s| s.id == lookup.skill_id) else {
        return Ok((
            AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Unavailable,
                message: format!("lookup skill '{}' is not declared", lookup.skill_id),
            },
            Vec::new(),
        ));
    };

    let out_dir = lookup_provider_output_dir(project_root, run_id, &provider_id);
    fs::create_dir_all(&out_dir)?;
    let rendered_args = render_lookup_skill_args(&skill.args, keyword, &out_dir);
    let args = split_command_args(&rendered_args);
    let mut command = Command::new(&exec);
    command.args(&args);
    command.current_dir(&out_dir);
    command.kill_on_drop(true);

    let timeout = Duration::from_secs(lookup.timeout_secs.clamp(1, 300));
    tracing::info!(
        provider = %provider_id,
        run_id,
        timeout_secs = timeout.as_secs(),
        keyword,
        "running asset_intel lookup cli provider"
    );

    let output_result = tokio::time::timeout(timeout, command.output()).await;
    let output = match output_result {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            tracing::warn!(provider = %provider_id, error = %err, "lookup spawn failed");
            return Ok((
                AssetIntelProviderRunStatus {
                    provider_id: provider_id.clone(),
                    status: AssetIntelProviderRunState::Unavailable,
                    message: format!("spawn failed: {err}"),
                },
                Vec::new(),
            ));
        }
        Err(_) => {
            tracing::warn!(
                provider = %provider_id,
                timeout_secs = timeout.as_secs(),
                "lookup timed out"
            );
            return Ok((
                AssetIntelProviderRunStatus {
                    provider_id: provider_id.clone(),
                    status: AssetIntelProviderRunState::Failed,
                    message: format!("lookup timed out after {}s", timeout.as_secs()),
                },
                Vec::new(),
            ));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let preview: String = stdout.chars().take(512).collect();
    let mut matches: Vec<LookupCompanyMatch> = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            matches.extend(extract_lookup_matches(&provider_id, lookup, &value));
        }
    }
    let mut files = Vec::new();
    collect_json_files(&out_dir, &mut files)?;
    files.sort();
    for path in files {
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(value) = serde_json::from_str::<Value>(&raw) {
                matches.extend(extract_lookup_matches(&provider_id, lookup, &value));
            }
        }
    }

    if !output.status.success() {
        return Ok((
            AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!(
                    "lookup exited with code {:?}: {preview}",
                    output.status.code()
                ),
            },
            matches,
        ));
    }

    if matches.is_empty() {
        return Ok((
            AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::CheckedEmpty,
                message: format!("'{provider_id}' lookup found no matches"),
            },
            matches,
        ));
    }

    Ok((
        AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Completed,
            message: format!(
                "'{provider_id}' lookup returned {} match(es)",
                matches.len()
            ),
        },
        matches,
    ))
}

#[tauri::command]
pub async fn asset_intel_list_providers(
    pentest: tauri::State<'_, PentestState>,
) -> Result<Vec<AssetIntelProviderDescriptor>, GolishError> {
    let pentest_config = pentest.config_manager.get().await;
    let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }
    Ok(provider_descriptors_from_tools(&scan.tools))
}

/// Hard cap so frontend lookup modals stay scannable. Per-provider lookups
/// can exceed this individually; we trim after dedupe.
const LOOKUP_RESULTS_HARD_CAP: usize = 25;

#[tauri::command]
pub async fn asset_intel_lookup_company(
    state: tauri::State<'_, DbState>,
    pentest: tauri::State<'_, PentestState>,
    args: AssetIntelLookupRequest,
) -> Result<AssetIntelLookupResult, GolishError> {
    let _ = state.pool_ready().await?;
    if args.keyword.trim().is_empty() {
        return Err(GolishError::Validation(
            "keyword is required for asset intel lookup".into(),
        ));
    }

    let pentest_config = pentest.config_manager.get().await;
    let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }

    // Select providers: explicit ids if given (must exist + have lookup),
    // otherwise every tool with a lookup descriptor regardless of `auto`.
    // Lookup is meant for "I want to disambiguate" so we don't apply the
    // auto.priority filter — the user has already opted in by clicking
    // "Look up company".
    let selected: Vec<&ToolConfig> = if args.provider_ids.is_empty() {
        scan.tools
            .iter()
            .filter(|t| {
                t.asset_intel
                    .as_ref()
                    .and_then(|a| a.lookup.as_ref())
                    .is_some_and(|l| l.enabled)
            })
            .collect()
    } else {
        let mut out = Vec::new();
        for provider_id in &args.provider_ids {
            let Some(tool) = scan
                .tools
                .iter()
                .find(|t| provider_id_for_tool(t).as_deref() == Some(provider_id.as_str()))
            else {
                return Err(GolishError::NotFound(format!(
                    "asset intel provider '{provider_id}' is not registered"
                )));
            };
            out.push(tool);
        }
        out
    };

    if selected.is_empty() {
        return Err(GolishError::Validation(
            "no asset_intel provider with an enabled lookup descriptor is available".into(),
        ));
    }

    let run_id = Uuid::new_v4().to_string();
    // Lookup writes nothing to organizations.intel; output is a per-call
    // scratch dir keyed by run_id so concurrent lookups don't collide.
    let project_root = pentest_config.tools_dir.clone();

    let mut provider_status = Vec::new();
    let mut all_matches = Vec::new();
    for tool in selected {
        let (status, matches) = run_lookup_cli_provider(
            tool,
            &scan.tools,
            &pentest_config.tools_dir,
            &project_root,
            &run_id,
            args.keyword.trim(),
        )
        .await?;
        provider_status.push(status);
        all_matches.extend(matches);
    }

    let mut deduped = dedupe_lookup_matches(all_matches);
    deduped.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let limit = args
        .limit
        .unwrap_or(LOOKUP_RESULTS_HARD_CAP)
        .min(LOOKUP_RESULTS_HARD_CAP);
    deduped.truncate(limit);

    Ok(AssetIntelLookupResult {
        run_id,
        matches: deduped,
        provider_status,
    })
}

/// Legacy single-shot hydrate command.
///
/// Runs **every** auto-default provider against the given organization with
/// the same `company_name` input. Kept for backward compatibility with older
/// frontend callers and tests. New code should prefer the two-phase
/// orchestration commands:
/// - [`asset_intel_hydrate_subsidiaries`] for the discovery phase
/// - [`asset_intel_enrich_organization`] / [`asset_intel_enrich_batch`] for
///   the enrichment phase
///
/// See `docs/design/2026-05-22-asset-intel-two-phase-hydrate.md` for the
/// rationale.
#[tauri::command]
pub async fn asset_intel_hydrate(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    pentest: tauri::State<'_, PentestState>,
    args: AssetIntelHydrateArgs,
) -> Result<AssetIntelRun, GolishError> {
    let pool = state.pool_ready().await?;
    let organization_id: Uuid = args.organization_id.parse()?;
    let row = golish_db::repo::organizations::get_one(pool, organization_id)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {}", args.organization_id)))?;
    let company_name = args.company_name.unwrap_or_else(|| row.name.clone());
    if company_name.trim().is_empty() {
        return Err(GolishError::Validation(
            "company_name is required for asset intel hydrate".into(),
        ));
    }

    let pentest_config = pentest.config_manager.get().await;
    let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }

    let selected = select_asset_intel_providers(&scan.tools, &args.provider_ids)?;
    let sink = TauriEventEmitter::handle(app);

    run_providers_for_org(
        Some(&sink),
        pool,
        &pentest_config,
        &scan.tools,
        selected,
        &row,
        &company_name,
        &enrichment_hydrate_config(args.config),
    )
    .await
}

/// Tauri command · two-phase hydrate **discovery** entrypoint.
///
/// Runs only the providers that declare a `subsidiaries` capability
/// (currently enscan-go) against the master organization, then writes the
/// resulting child-org / target candidates back under the **master
/// organization's** candidate list — exactly like the legacy single-shot
/// hydrate, but with 0.zone-style enrichment providers held back for the
/// later enrich phase.
///
/// Frontend wires this to the master row's "查子公司" button.
#[tauri::command]
pub async fn asset_intel_hydrate_subsidiaries(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    pentest: tauri::State<'_, PentestState>,
    args: AssetIntelHydrateArgs,
) -> Result<AssetIntelRun, GolishError> {
    let pool = state.pool_ready().await?;
    let organization_id: Uuid = args.organization_id.parse()?;
    let row = golish_db::repo::organizations::get_one(pool, organization_id)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {}", args.organization_id)))?;
    let company_name = args.company_name.unwrap_or_else(|| row.name.clone());
    if company_name.trim().is_empty() {
        return Err(GolishError::Validation(
            "company_name is required for asset intel hydrate subsidiaries".into(),
        ));
    }

    let pentest_config = pentest.config_manager.get().await;
    let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }

    let selected = select_subsidiary_providers(&scan.tools, &args.provider_ids)?;
    if selected.is_empty() {
        return Err(GolishError::Validation(
            "no asset intel provider with a 'subsidiaries' capability is available".into(),
        ));
    }
    let sink = TauriEventEmitter::handle(app);

    let discovery_config = discovery_hydrate_config(args.config);
    let discovery_policy = selected
        .iter()
        .filter_map(|tool| tool.asset_intel.as_ref())
        .find(|asset| asset.discovery.auto_promote)
        .map(|asset| asset.discovery.clone())
        .unwrap_or_default();
    let mut run = run_providers_for_org(
        Some(&sink),
        pool,
        &pentest_config,
        &scan.tools,
        selected,
        &row,
        &company_name,
        &discovery_config,
    )
    .await?;
    if discovery_policy.auto_promote {
        let promotion =
            auto_promote_discovered_children(pool, &row, &run.candidates, &discovery_policy)
                .await?;
        run.evidence.push(promotion);
        run.candidates = OrganizationCandidates::default();
    }
    Ok(run)
}

/// Args for [`asset_intel_enrich_organization`].
///
/// Differences vs. [`AssetIntelHydrateArgs`]: no `company_name` override —
/// enrichment always uses the canonical `organization.name` so that
/// querying 0.zone for "中国平安" enriches the master org, while querying
/// for "平安银行" enriches that specific child. Letting callers override
/// the name would defeat the whole purpose of the two-phase split.
/// Tauri command · two-phase hydrate **enrichment** entrypoint (single org).
///
/// Runs only providers that do **not** declare a `subsidiaries` capability
/// (currently 0.zone et al.) against the given organization, using
/// `organization.name` as the query input. Candidates and master-record
/// profile updates land on the targeted org — not on its parent.
///
/// Frontend wires this to per-org "补字段" buttons.
#[tauri::command]
pub async fn asset_intel_enrich_organization(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    pentest: tauri::State<'_, PentestState>,
    args: AssetIntelEnrichOrganizationArgs,
) -> Result<AssetIntelRun, GolishError> {
    let pool = state.pool_ready().await?;
    let organization_id: Uuid = args.organization_id.parse()?;
    let row = golish_db::repo::organizations::get_one(pool, organization_id)
        .await?
        .ok_or_else(|| GolishError::NotFound(format!("organization {}", args.organization_id)))?;
    let company_name = row.name.clone();
    if company_name.trim().is_empty() {
        return Err(GolishError::Validation(
            "organization name is empty; cannot run enrichment".into(),
        ));
    }

    let pentest_config = pentest.config_manager.get().await;
    let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }

    let selected = select_enrichment_providers(&scan.tools, &args.provider_ids)?;
    if selected.is_empty() {
        return Err(GolishError::Validation(
            "no asset intel enrichment provider (non-subsidiaries) is available".into(),
        ));
    }
    let sink = TauriEventEmitter::handle(app);

    run_providers_for_org(
        Some(&sink),
        pool,
        &pentest_config,
        &scan.tools,
        selected,
        &row,
        &company_name,
        &args.config,
    )
    .await
}

/// Tauri command · two-phase hydrate **enrichment** entrypoint (batch).
///
/// Resolves the parent organization, optionally includes it as the first
/// run, then iterates over every direct child (matched by
/// `parent_id = parent_organization_id`) and runs the enrichment provider
/// set against each in turn. Failures in one org don't abort the batch —
/// they just produce a `Failed` / `Partial` `AssetIntelRun` in the result
/// vector. Orgs with an empty `name` are skipped, not failed.
///
/// Frontend wires this to the master row's "批量补字段" button.
#[tauri::command]
pub async fn asset_intel_enrich_batch(
    app: tauri::AppHandle,
    state: tauri::State<'_, DbState>,
    pentest: tauri::State<'_, PentestState>,
    args: AssetIntelEnrichBatchArgs,
) -> Result<AssetIntelEnrichBatchResult, GolishError> {
    let pool = state.pool_ready().await?;
    let parent_id: Uuid = args.parent_organization_id.parse()?;
    let parent_row = golish_db::repo::organizations::get_one(pool, parent_id)
        .await?
        .ok_or_else(|| {
            GolishError::NotFound(format!("organization {}", args.parent_organization_id))
        })?;

    let pentest_config = pentest.config_manager.get().await;
    let scan = golish_pentest::scan_toolsconfig(&pentest_config.toolsconfig_dir);
    if !scan.success {
        return Err(GolishError::Internal(
            scan.error
                .unwrap_or_else(|| "toolsconfig scan failed".into()),
        ));
    }

    let selected_for_check = select_enrichment_providers(&scan.tools, &args.provider_ids)?;
    if selected_for_check.is_empty() {
        return Err(GolishError::Validation(
            "no asset intel enrichment provider (non-subsidiaries) is available".into(),
        ));
    }

    // Build target list: parent first (when requested), then children in the
    // same order organizations::list returns (parent_id NULLS FIRST → sort_order
    // → name). We re-fetch the parent row from the same list to keep IDs and
    // intel snapshots fresh inside the loop.
    let all_orgs = golish_db::repo::organizations::list(pool, &parent_row.project_path).await?;
    let include_self = args.include_self.unwrap_or(true);
    let mut targets: Vec<golish_db::models::Organization> = Vec::new();
    if include_self {
        targets.push(parent_row.clone());
    }
    for org in &all_orgs {
        if org.parent_id == Some(parent_id) {
            targets.push(org.clone());
        }
    }

    if targets.is_empty() {
        return Err(GolishError::Validation(format!(
            "organization {} has no children and include_self=false; nothing to enrich",
            args.parent_organization_id
        )));
    }

    let sink = TauriEventEmitter::handle(app);
    let mut runs: Vec<AssetIntelRun> = Vec::new();
    let mut skipped: Vec<AssetIntelEnrichBatchSkip> = Vec::new();
    for org in targets {
        let company_name = org.name.trim().to_string();
        if company_name.is_empty() {
            skipped.push(AssetIntelEnrichBatchSkip {
                organization_id: org.id.to_string(),
                reason: "empty_name".into(),
            });
            continue;
        }
        // Re-select providers per iteration so that hot-reloading toolsconfig
        // (e.g. operator disabling 0.zone mid-batch) doesn't keep firing the
        // disabled provider. Conservative but cheap: the scan was already
        // performed once above.
        let selected = match select_enrichment_providers(&scan.tools, &args.provider_ids) {
            Ok(p) if !p.is_empty() => p,
            Ok(_) => {
                skipped.push(AssetIntelEnrichBatchSkip {
                    organization_id: org.id.to_string(),
                    reason: "no_enrichment_provider".into(),
                });
                continue;
            }
            Err(err) => {
                skipped.push(AssetIntelEnrichBatchSkip {
                    organization_id: org.id.to_string(),
                    reason: format!("provider_select_error: {err}"),
                });
                continue;
            }
        };
        match run_providers_for_org(
            Some(&sink),
            pool,
            &pentest_config,
            &scan.tools,
            selected,
            &org,
            &company_name,
            &enrichment_hydrate_config(args.config.clone()),
        )
        .await
        {
            Ok(run) => runs.push(run),
            Err(err) => {
                // Don't abort the batch; record skip and continue. This keeps
                // a 0.zone quota-exhausted error from killing enrichment for
                // every later org.
                skipped.push(AssetIntelEnrichBatchSkip {
                    organization_id: org.id.to_string(),
                    reason: format!("run_failed: {err}"),
                });
            }
        }
    }

    Ok(AssetIntelEnrichBatchResult { runs, skipped })
}

/// Run a set of asset-intel providers against a single organization, writing
/// candidates + master-record profile fields back to **that org's** id.
///
/// This is the shared backbone behind every hydrate / enrich command:
/// - legacy [`asset_intel_hydrate`] passes the full provider list,
/// - [`asset_intel_hydrate_subsidiaries`] passes the discovery subset,
/// - [`asset_intel_enrich_organization`] and [`asset_intel_enrich_batch`]
///   pass the enrichment subset (and use the org's own name as the query).
///
/// The function intentionally takes already-filtered `providers` so the
/// caller controls phase semantics; this body only knows "run these tools
/// for this org with this name".
#[allow(clippy::too_many_arguments)]
async fn run_providers_for_org(
    sink: Option<&EventEmitterHandle>,
    pool: &sqlx::PgPool,
    pentest_config: &golish_pentest::config::PentestConfig,
    scan_tools: &[ToolConfig],
    providers: Vec<ToolConfig>,
    org_row: &golish_db::models::Organization,
    company_name: &str,
    config: &AssetIntelHydrateConfig,
) -> Result<AssetIntelRun, GolishError> {
    let run_id = Uuid::new_v4().to_string();
    let project_root = PathBuf::from(&org_row.project_path);
    let organization_id = org_row.id;

    let mut provider_status = Vec::new();
    let mut evidence = Vec::new();
    let mut candidates = OrganizationCandidates::default();
    let mut profile_entries: Vec<ProfileFieldEntry> = Vec::new();
    for tool in &providers {
        let asset = tool.asset_intel.as_ref().ok_or_else(|| {
            GolishError::Validation(format!("tool '{}' has no asset_intel descriptor", tool.id))
        })?;
        let (status, next_candidates, next_evidence, next_profile) = match &asset.runtime {
            golish_pentest::models::AssetIntelRuntimeConfig::CliJson { .. } => {
                run_cli_json_provider(
                    tool,
                    scan_tools,
                    &pentest_config.tools_dir,
                    &project_root,
                    &run_id,
                    company_name,
                    config,
                    sink,
                )
                .await?
            }
            golish_pentest::models::AssetIntelRuntimeConfig::HttpJson { .. } => {
                run_http_json_provider(pool, tool, &run_id, company_name, config, sink).await?
            }
        };
        evidence.push(next_evidence);
        if provider_output_is_trusted(&status) {
            merge_candidates(&mut candidates, next_candidates);
            profile_entries.extend(next_profile);
        }
        provider_status.push(status);
    }

    // Master record write happens *before* candidate upsert. If the patch is
    // empty (no descriptor profile_fields fired) we skip the DB roundtrip to
    // avoid noise. The patch is the merged view across every provider —
    // duplicate values are collapsed so the master record stays canonical.
    if !profile_entries.is_empty() {
        if let Some(mut patch) = build_profile_patch_from_entries(&org_row.intel, &profile_entries)?
        {
            merge_profile_patch_with_existing(org_row, &mut patch);
            golish_db::repo::organizations::update_profile(pool, organization_id, &patch).await?;
        }
    }

    if config.create_candidates.unwrap_or(true) {
        let flat = flatten_candidates(&candidates);
        if !flat.is_empty() {
            candidates =
                upsert_organization_candidates_for_org(pool, organization_id, flat).await?;
        }
    }
    let failed = provider_status
        .iter()
        .filter(|item| {
            matches!(
                item.status,
                AssetIntelProviderRunState::Failed | AssetIntelProviderRunState::Unavailable
            )
        })
        .count();
    let status = if failed == 0 {
        AssetIntelRunStatus::Completed
    } else if failed == provider_status.len() {
        AssetIntelRunStatus::Failed
    } else {
        AssetIntelRunStatus::Partial
    };

    Ok(AssetIntelRun {
        run_id,
        status,
        provider_status,
        candidates,
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_runtime() -> golish_pentest::models::AssetIntelRuntimeConfig {
        golish_pentest::models::AssetIntelRuntimeConfig::CliJson {
            skill_id: "company-default-json".into(),
            timeout_secs: 30,
            artifact_globs: vec![],
            arg_bindings: std::collections::HashMap::new(),
        }
    }

    fn fake_normalize_config() -> golish_pentest::models::AssetIntelNormalizeConfig {
        golish_pentest::models::AssetIntelNormalizeConfig {
            organization: vec![golish_pentest::models::AssetIntelNormalizeRule {
                path: "$..invest[*]".into(),
                label: golish_pentest::models::AssetIntelFieldRef::Field("name".into()),
                value: golish_pentest::models::AssetIntelFieldRef::Field("name".into()),
                confidence: 0.82,
                when: vec![],
            }],
            target: vec![golish_pentest::models::AssetIntelNormalizeRule {
                path: "$..icp[*]".into(),
                label: golish_pentest::models::AssetIntelFieldRef::Field("domain".into()),
                value: golish_pentest::models::AssetIntelFieldRef::FirstOf(vec![
                    "domain".into(),
                    "url".into(),
                ]),
                confidence: 0.78,
                when: vec![],
            }],
            profile_fields: vec![],
        }
    }

    fn org_candidate_with_raw(name: &str, scale: &str, status: &str) -> OrganizationCandidate {
        OrganizationCandidate {
            id: format!("org:enscan-go:{name}"),
            kind: OrganizationCandidateKind::Organization,
            label: name.into(),
            value: name.into(),
            source: "enscan-go".into(),
            confidence: 0.82,
            status: "needs_review".into(),
            evidence: serde_json::json!({
                "provider": "enscan-go",
                "runId": "run-test",
                "raw": {
                    "name": name,
                    "scale": scale,
                    "status": status,
                    "pid": format!("pid-{name}")
                }
            }),
            created_at: 1,
        }
    }

    fn auto_promote_policy() -> golish_pentest::models::AssetIntelDiscoveryConfig {
        use golish_pentest::models::{AssetIntelNormalizeFilter, AssetIntelNormalizeFilterOp};
        golish_pentest::models::AssetIntelDiscoveryConfig {
            auto_promote: true,
            promote_when: vec![
                AssetIntelNormalizeFilter {
                    field: "scale".into(),
                    op: AssetIntelNormalizeFilterOp::Gte,
                    value: "51".into(),
                },
                AssetIntelNormalizeFilter {
                    field: "status".into(),
                    op: AssetIntelNormalizeFilterOp::Contains,
                    value: "开业".into(),
                },
            ],
            ownership_field: "scale".into(),
            dedupe_by: vec!["pid".into(), "name".into()],
        }
    }

    #[test]
    fn provider_output_is_trusted_only_for_successful_terminal_states() {
        assert!(provider_output_is_trusted(&AssetIntelProviderRunStatus {
            provider_id: "enscan-go".into(),
            status: AssetIntelProviderRunState::Completed,
            message: "ok".into(),
        }));
        assert!(provider_output_is_trusted(&AssetIntelProviderRunStatus {
            provider_id: "enscan-go".into(),
            status: AssetIntelProviderRunState::CheckedEmpty,
            message: "empty".into(),
        }));
        assert!(!provider_output_is_trusted(&AssetIntelProviderRunStatus {
            provider_id: "enscan-go-tyc-discovery".into(),
            status: AssetIntelProviderRunState::Failed,
            message: "command failed after emitting partial stdout".into(),
        }));
        assert!(!provider_output_is_trusted(&AssetIntelProviderRunStatus {
            provider_id: "enscan-go-kc-discovery".into(),
            status: AssetIntelProviderRunState::Unavailable,
            message: "missing credentials".into(),
        }));
    }

    #[test]
    fn asset_intel_provider_descriptors_load_from_tool_configs() {
        let tool = golish_pentest::models::ToolConfig {
            id: "fake-intel".into(),
            name: "Fake Intel".into(),
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "fake-provider".into(),
                display_name: "Fake Provider".into(),
                capabilities: vec!["domains".into(), "apps".into()],
                requires_integration: Some(
                    golish_pentest::models::AssetIntelIntegrationRequirement {
                        tool_id: "fake-intel".into(),
                        group_ids: vec!["default".into()],
                    },
                ),
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: true,
                    priority: 10,
                },
                runtime: fake_runtime(),
                normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                lookup: None,
            }),
            ..Default::default()
        };

        let providers = provider_descriptors_from_tools(&[tool]);

        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "fake-provider");
        assert_eq!(providers[0].display_name, "Fake Provider");
        assert_eq!(
            providers[0].requires_integration,
            Some(AssetIntelIntegrationRequirement {
                tool_id: "fake-intel".into(),
                group_ids: vec!["default".into()],
            })
        );
        assert!(providers[0]
            .capabilities
            .contains(&AssetIntelCapability::Domains));
        assert!(providers[0]
            .capabilities
            .contains(&AssetIntelCapability::Apps));
    }

    #[test]
    fn normalize_provider_records_splits_candidates_and_preserves_evidence() {
        let candidates = normalize_provider_records(
            "mock",
            "run-1",
            123,
            vec![
                AssetIntelProviderRecord {
                    kind: OrganizationCandidateKind::Organization,
                    label: "Acme Subsidiary".into(),
                    value: "Acme Subsidiary".into(),
                    confidence: 0.86,
                    evidence: serde_json::json!({"raw": {"ownership": 51}}),
                },
                AssetIntelProviderRecord {
                    kind: OrganizationCandidateKind::Target,
                    label: "api.acme.test".into(),
                    value: "api.acme.test".into(),
                    confidence: 0.72,
                    evidence: serde_json::json!({"raw": {"type": "domain"}}),
                },
            ],
        );

        assert_eq!(candidates.organizations.len(), 1);
        assert_eq!(candidates.targets.len(), 1);
        assert_eq!(candidates.organizations[0].source, "mock");
        assert_eq!(candidates.organizations[0].status, "needs_review");
        assert_eq!(candidates.organizations[0].created_at, 123);
        assert_eq!(candidates.organizations[0].evidence["provider"], "mock");
        assert_eq!(candidates.organizations[0].evidence["runId"], "run-1");
        assert_eq!(candidates.targets[0].id, "target:mock:api.acme.test");
    }

    #[test]
    fn auto_promote_child_decisions_only_promote_active_controlled_investments() {
        let candidates = OrganizationCandidates {
            organizations: vec![
                org_candidate_with_raw("平安信托有限责任公司", "99.880923%", "开业"),
                org_candidate_with_raw("平安证券股份有限公司", "40.9596%", "开业"),
                org_candidate_with_raw("注销分支", "100%", "注销"),
                org_candidate_with_raw("已存在子公司", "100%", "开业"),
            ],
            targets: vec![],
        };
        let existing = HashSet::from(["已存在子公司".to_string()]);
        let policy = auto_promote_policy();

        let decisions = auto_promote_child_decisions(&candidates, &policy, &existing);

        assert_eq!(decisions.iter().filter(|item| item.promote).count(), 1);
        assert_eq!(decisions[0].candidate.value, "平安信托有限责任公司");
        assert_eq!(decisions[0].ownership_percent, Some(99.880923));
        assert_eq!(
            decisions
                .iter()
                .filter_map(|item| item.reason.as_ref())
                .collect::<Vec<_>>(),
            vec![
                &AutoPromoteSkipReason::OwnershipBelowThreshold,
                &AutoPromoteSkipReason::InactiveStatus,
                &AutoPromoteSkipReason::Duplicate,
            ]
        );
    }

    #[test]
    fn clear_engagement_candidates_preserves_engagement_metadata() {
        let intel = serde_json::json!({
            "engagement": {
                "mode": "discover_assets",
                "lookup_match": { "name": "中国平安保险（集团）股份有限公司" },
                "candidates": {
                    "organizations": [{ "id": "org:enscan-go:old", "value": "old" }],
                    "targets": [{ "id": "target:enscan-go:old", "value": "old.example" }]
                }
            },
            "contacts": {
                "email": ["ir@example.test"]
            }
        });

        let cleared = clear_engagement_candidates_from_intel(intel).unwrap();

        assert_eq!(cleared["engagement"]["mode"], "discover_assets");
        assert_eq!(
            cleared["engagement"]["lookup_match"]["name"],
            "中国平安保险（集团）股份有限公司"
        );
        assert!(cleared["engagement"].get("candidates").is_none());
        assert_eq!(cleared["contacts"]["email"][0], "ir@example.test");
    }

    #[test]
    fn json_descriptor_normalizer_maps_nested_candidate_buckets() {
        let normalize = fake_normalize_config();
        let raw = serde_json::json!({
            "payload": {
                "invest": [{ "name": "小米科技有限责任公司" }],
                "icp": [{ "domain": "mi.com" }]
            }
        });

        let (candidates, profile) =
            normalize_json_with_descriptor("fake", "run-1", 123, &normalize, &raw);

        assert_eq!(candidates.organizations.len(), 1);
        assert_eq!(candidates.organizations[0].label, "小米科技有限责任公司");
        assert_eq!(candidates.organizations[0].source, "fake");
        assert_eq!(candidates.targets.len(), 1);
        assert_eq!(candidates.targets[0].value, "mi.com");
        assert_eq!(candidates.targets[0].confidence, 0.78);
        assert_eq!(candidates.targets[0].evidence["provider"], "fake");
        assert!(profile.is_empty(), "no profile_fields rules in fake config");
    }

    #[test]
    fn fake_provider_json_data_dedupes_across_sources() {
        let normalize = fake_normalize_config();
        let first_raw = serde_json::json!({
            "payload": {
                "invest": [{ "name": "小米科技有限责任公司" }],
                "icp": [{ "domain": "mi.com" }, { "domain": "api.mi.com" }]
            }
        });
        let second_raw = serde_json::json!({
            "data": {
                "invest": [{ "name": "小米科技有限责任公司" }],
                "icp": [{ "domain": "MI.COM" }, { "domain": "store.mi.com" }]
            }
        });

        let (mut merged, _) =
            normalize_json_with_descriptor("fake-cli", "run-1", 1, &normalize, &first_raw);
        let (http_candidates, _) =
            normalize_json_with_descriptor("fake-http", "run-1", 2, &normalize, &second_raw);
        merge_candidates(&mut merged, http_candidates);

        assert_eq!(merged.organizations.len(), 1);
        assert_eq!(merged.organizations[0].source, "fake-cli");
        assert_eq!(
            merged
                .targets
                .iter()
                .map(|item| item.value.as_str())
                .collect::<Vec<_>>(),
            vec!["mi.com", "api.mi.com", "store.mi.com"]
        );
    }

    #[tokio::test]
    async fn http_json_runtime_posts_fake_data_and_normalizes_candidates() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let mut bytes = Vec::new();
            loop {
                let n = stream.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                let req = String::from_utf8_lossy(&bytes);
                if req.contains("query_type=domain") {
                    break;
                }
            }
            let req = String::from_utf8_lossy(&bytes);
            assert!(req.starts_with("POST / HTTP/1.1"));
            assert!(req.contains("query=%E5%B0%8F%E7%B1%B3"));
            assert!(req.contains("query_type=domain"));

            let body = serde_json::json!({
                "code": 0,
                "data": [
                    { "domain": "mi.com", "title": "Xiaomi" },
                    { "domain": "api.mi.com", "title": "Xiaomi API" }
                ],
                "message": "ok"
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let mut form = std::collections::HashMap::new();
        form.insert("query".to_string(), "{{company_name}}".to_string());
        form.insert("query_type".to_string(), "domain".to_string());
        let tool = ToolConfig {
            id: "fake-http".into(),
            name: "Fake HTTP".into(),
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "fake-http".into(),
                display_name: "Fake HTTP".into(),
                capabilities: vec!["domains".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: true,
                    priority: 1,
                },
                runtime: golish_pentest::models::AssetIntelRuntimeConfig::HttpJson {
                    requests: vec![golish_pentest::models::AssetIntelHttpRequest {
                        id: "domains".into(),
                        method: "POST".into(),
                        url,
                        headers: std::collections::HashMap::new(),
                        form,
                        json: Value::Null,
                        timeout_secs: 5,
                    }],
                },
                normalize: golish_pentest::models::AssetIntelNormalizeConfig {
                    organization: vec![],
                    target: vec![golish_pentest::models::AssetIntelNormalizeRule {
                        path: "$..data[*]".into(),
                        label: golish_pentest::models::AssetIntelFieldRef::Field("title".into()),
                        value: golish_pentest::models::AssetIntelFieldRef::Field("domain".into()),
                        confidence: 0.72,
                        when: vec![],
                    }],
                    profile_fields: vec![],
                },
                discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                lookup: None,
            }),
            ..Default::default()
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://golish:golish@127.0.0.1:1/golish")
            .unwrap();

        let (status, candidates, evidence, _profile) = run_http_json_provider(
            &pool,
            &tool,
            "run-1",
            "小米",
            &AssetIntelHydrateConfig::default(),
            None,
        )
        .await
        .unwrap();
        server.await.unwrap();

        assert_eq!(status.status, AssetIntelProviderRunState::Completed);
        assert_eq!(candidates.targets.len(), 2);
        assert_eq!(candidates.targets[0].label, "Xiaomi");
        assert_eq!(candidates.targets[0].value, "mi.com");
        assert_eq!(evidence["candidateCount"], 2);
    }

    #[derive(Debug, Default, Clone)]
    struct RecordingEmitter {
        events: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    }

    impl golish_core::EventEmitter for RecordingEmitter {
        fn emit_json(&self, event: &str, payload: Value) {
            self.events
                .lock()
                .unwrap()
                .push((event.to_string(), payload));
        }
    }

    impl RecordingEmitter {
        fn snapshot(&self) -> Vec<(String, Value)> {
            self.events.lock().unwrap().clone()
        }

        fn handle(&self) -> EventEmitterHandle {
            EventEmitterHandle::new(self.clone())
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_json_runtime_streams_progress_and_artifact_batches() {
        use std::os::unix::fs::PermissionsExt;

        let tools_dir = tempfile::tempdir().unwrap();
        let project_root = tempfile::tempdir().unwrap();
        let executable = tools_dir.path().join("fake-asset-intel.sh");
        // Fake CLI:
        //   1) emit a progress line on stdout (non-JSON → progress event)
        //   2) write icp.json → artifact watcher should observe it
        //   3) sleep > ARTIFACT_POLL_INTERVAL so the watcher polls
        //   4) write app.json → another artifact batch
        //   5) emit another progress line + exit 0
        fs::write(
            &executable,
            r#"#!/bin/sh
echo "[stage] collecting icp"
printf '%s' '{"payload":{"icp":[{"domain":"a.example"}]}}' > "$(pwd)/icp.json"
sleep 0.8
echo "[stage] collecting app"
printf '%s' '{"payload":{"icp":[{"domain":"b.example"}]}}' > "$(pwd)/app.json"
sleep 0.8
echo "[stage] done"
"#,
        )
        .unwrap();
        let mut perms = fs::metadata(&executable).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&executable, perms).unwrap();

        let tool = ToolConfig {
            id: "fake-stream".into(),
            name: "Fake Stream".into(),
            executable: "fake-asset-intel.sh".into(),
            runtime: "native".into(),
            skills: vec![golish_pentest::models::ToolSkill {
                id: "company-default-json".into(),
                name: "Company JSON".into(),
                description: String::new(),
                args: String::new(),
                tags: vec![],
            }],
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "fake-stream".into(),
                display_name: "Fake Stream".into(),
                capabilities: vec!["domains".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: true,
                    priority: 1,
                },
                runtime: fake_runtime(),
                normalize: fake_normalize_config(),
                discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                lookup: None,
            }),
            ..Default::default()
        };

        let recorder = RecordingEmitter::default();
        let handle = recorder.handle();
        let (status, candidates, _evidence, _profile) = run_cli_json_provider(
            &tool,
            std::slice::from_ref(&tool),
            tools_dir.path(),
            project_root.path(),
            "run-stream",
            "Acme",
            &AssetIntelHydrateConfig::default(),
            Some(&handle),
        )
        .await
        .unwrap();

        assert_eq!(status.status, AssetIntelProviderRunState::Completed);
        // dedup of a.example + b.example
        assert_eq!(candidates.targets.len(), 2);

        let events = recorder.snapshot();
        let names: Vec<&str> = events
            .iter()
            .filter_map(|(name, payload)| {
                if name == ASSET_INTEL_EVENT {
                    payload.get("kind").and_then(|v| v.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(
            names.iter().any(|name| *name == "provider_started"),
            "expected provider_started in {:?}",
            names
        );
        assert!(
            names
                .iter()
                .filter(|name| **name == "provider_progress")
                .count()
                >= 2,
            "expected at least 2 progress events (saw {:?})",
            names
        );
        let batch_events: Vec<&(String, Value)> = events
            .iter()
            .filter(|(_, payload)| {
                payload.get("kind").and_then(|v| v.as_str()) == Some("provider_batch")
            })
            .collect();
        assert!(
            !batch_events.is_empty(),
            "expected at least one provider_batch event (got events: {:?})",
            names
        );
        // every batch should carry source = "artifact" with an artifact path
        for (_, payload) in &batch_events {
            assert_eq!(
                payload.get("source").and_then(|v| v.as_str()),
                Some("artifact"),
                "batch should originate from artifact (payload={:?})",
                payload
            );
            assert!(
                payload
                    .get("artifact")
                    .and_then(|v| v.as_str())
                    .map(|p| p.ends_with(".json"))
                    .unwrap_or(false),
                "artifact path should be set (payload={:?})",
                payload
            );
        }
        assert!(
            names.iter().any(|name| *name == "provider_completed"),
            "expected provider_completed in {:?}",
            names
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_json_runtime_runs_in_project_tool_output_dir() {
        use std::os::unix::fs::PermissionsExt;

        let tools_dir = tempfile::tempdir().unwrap();
        let project_root = tempfile::tempdir().unwrap();
        let executable = tools_dir.path().join("fake-asset-intel.sh");
        fs::write(
            &executable,
            r#"#!/bin/sh
case "$(pwd)" in
  */.golish/tool-output/asset-intel/run-cwd/fake-cli)
    printf '{"payload":{"icp":[{"domain":"cwd.example","title":"CWD OK"}]}}'
    ;;
  *)
    echo "bad cwd: $(pwd)" >&2
    exit 2
    ;;
esac
"#,
        )
        .unwrap();
        let mut perms = fs::metadata(&executable).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&executable, perms).unwrap();

        let tool = ToolConfig {
            id: "fake-cli".into(),
            name: "Fake CLI".into(),
            executable: "fake-asset-intel.sh".into(),
            runtime: "native".into(),
            skills: vec![golish_pentest::models::ToolSkill {
                id: "company-default-json".into(),
                name: "Company JSON".into(),
                description: String::new(),
                args: String::new(),
                tags: vec![],
            }],
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "fake-cli".into(),
                display_name: "Fake CLI".into(),
                capabilities: vec!["domains".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: true,
                    priority: 1,
                },
                runtime: fake_runtime(),
                normalize: fake_normalize_config(),
                discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                lookup: None,
            }),
            ..Default::default()
        };

        let (status, candidates, evidence, _profile) = run_cli_json_provider(
            &tool,
            std::slice::from_ref(&tool),
            tools_dir.path(),
            project_root.path(),
            "run-cwd",
            "Acme",
            &AssetIntelHydrateConfig::default(),
            None,
        )
        .await
        .unwrap();

        assert_eq!(status.status, AssetIntelProviderRunState::Completed);
        assert_eq!(candidates.targets.len(), 1);
        assert_eq!(candidates.targets[0].value, "cwd.example");
        assert!(
            evidence["outDir"].as_str().is_some_and(
                |path| path.ends_with(".golish/tool-output/asset-intel/run-cwd/fake-cli")
            )
        );
    }

    #[test]
    fn asset_intel_skill_args_render_config_bindings() {
        let mut bindings = std::collections::HashMap::new();
        bindings.insert(
            "min_ownership_percent".to_string(),
            "-invest {{config.min_ownership_percent}}".to_string(),
        );
        bindings.insert("depth".to_string(), "-deep {{config.depth}}".to_string());
        bindings.insert("include_branches".to_string(), "-branch".to_string());

        let rendered = render_asset_intel_skill_args(
            "-n \"{{org}}\" -json -out-dir \"{{out_dir}}\"",
            "小米",
            &PathBuf::from("/tmp/golish-enscan"),
            &AssetIntelHydrateConfig {
                min_ownership_percent: Some("51".into()),
                depth: Some("2".into()),
                include_branches: Some(true),
                create_candidates: Some(true),
            },
            &bindings,
        );

        assert_eq!(
            split_command_args(&rendered),
            vec![
                "-n",
                "小米",
                "-json",
                "-out-dir",
                "/tmp/golish-enscan",
                "-invest",
                "51",
                "-deep",
                "2",
                "-branch",
            ]
        );
    }

    #[test]
    fn select_asset_intel_providers_uses_json_auto_priority() {
        fn tool(id: &str, priority: i32, enabled: bool) -> ToolConfig {
            ToolConfig {
                id: id.into(),
                name: id.into(),
                asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                    enabled: true,
                    provider_id: id.into(),
                    display_name: id.into(),
                    capabilities: vec!["domains".into()],
                    requires_integration: None,
                    auto: golish_pentest::models::AssetIntelAutoConfig {
                        default: enabled,
                        priority,
                    },
                    runtime: fake_runtime(),
                    normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                    discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                    lookup: None,
                }),
                ..Default::default()
            }
        }

        let tools = vec![
            tool("low", 10, true),
            tool("high", 100, true),
            tool("off", 200, false),
        ];
        let selected = select_asset_intel_providers(&tools, &[]).unwrap();

        assert_eq!(
            selected
                .iter()
                .map(|tool| provider_id_for_tool(tool).unwrap())
                .collect::<Vec<_>>(),
            vec!["high".to_string(), "low".to_string()]
        );
    }

    /// Shared fixture for two-phase selector tests: 3 providers covering
    /// the realistic mix we ship today.
    /// - `enscan-go`: subsidiaries + domains (discovery-capable)
    /// - `0.zone`:   domains + apps (enrichment-only)
    /// - `legacy`:   domains, auto.default=false (excluded by auto filter)
    fn two_phase_fixture_tools() -> Vec<ToolConfig> {
        fn tool(id: &str, caps: &[&str], priority: i32, auto_default: bool) -> ToolConfig {
            ToolConfig {
                id: id.into(),
                name: id.into(),
                asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                    enabled: true,
                    provider_id: id.into(),
                    display_name: id.into(),
                    capabilities: caps.iter().map(|s| (*s).to_string()).collect(),
                    requires_integration: None,
                    auto: golish_pentest::models::AssetIntelAutoConfig {
                        default: auto_default,
                        priority,
                    },
                    runtime: fake_runtime(),
                    normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                    discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                    lookup: None,
                }),
                ..Default::default()
            }
        }

        vec![
            tool("enscan-go", &["subsidiaries", "domains", "icp"], 100, true),
            tool("0.zone", &["domains", "apps", "contacts"], 90, true),
            tool("legacy", &["domains"], 50, false),
        ]
    }

    #[test]
    fn select_subsidiary_providers_keeps_only_subsidiaries_capable_tools() {
        let tools = two_phase_fixture_tools();
        let selected = select_subsidiary_providers(&tools, &[]).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|t| provider_id_for_tool(t).unwrap())
                .collect::<Vec<_>>(),
            vec!["enscan-go".to_string()],
            "only enscan-go declares the subsidiaries capability"
        );
    }

    fn multi_provider_tool(id: &str, providers: &[(&str, &[&str], bool, i32)]) -> ToolConfig {
        ToolConfig {
            id: id.into(),
            name: id.into(),
            executable: format!("{id}/bin"),
            asset_intel_providers: Some(
                providers
                    .iter()
                    .map(|(pid, caps, default, priority)| {
                        golish_pentest::models::AssetIntelToolConfig {
                            enabled: true,
                            provider_id: (*pid).into(),
                            display_name: (*pid).into(),
                            capabilities: caps.iter().map(|c| (*c).to_string()).collect(),
                            requires_integration: None,
                            auto: golish_pentest::models::AssetIntelAutoConfig {
                                default: *default,
                                priority: *priority,
                            },
                            runtime: fake_runtime(),
                            normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                            discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                            lookup: None,
                        }
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn select_subsidiary_providers_expands_multi_provider_tool() {
        let tool = multi_provider_tool(
            "multi",
            &[
                ("multi-hi", &["subsidiaries"], true, 100),
                ("multi-lo", &["subsidiaries"], true, 50),
            ],
        );
        let selected = select_subsidiary_providers(&[tool], &[]).unwrap();
        assert_eq!(selected.len(), 2);
        let ids: Vec<String> = selected
            .iter()
            .map(|t| provider_id_for_tool(t).unwrap())
            .collect();
        assert_eq!(ids, vec!["multi-hi".to_string(), "multi-lo".to_string()]);
    }

    #[test]
    fn select_asset_intel_providers_treats_multi_provider_tool_as_single_pool() {
        // Tool A has two providers (priority 50 / 100); tool B has one (priority 75).
        // Expected sort across both tools: [100, 75, 50].
        let tool_a = multi_provider_tool(
            "multi",
            &[
                ("multi-low", &["subsidiaries"], true, 50),
                ("multi-high", &["subsidiaries"], true, 100),
            ],
        );
        let tool_b = ToolConfig {
            id: "single".into(),
            name: "single".into(),
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "single-mid".into(),
                display_name: "single".into(),
                capabilities: vec!["subsidiaries".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: true,
                    priority: 75,
                },
                runtime: fake_runtime(),
                normalize: Default::default(),
                discovery: Default::default(),
                lookup: None,
            }),
            ..Default::default()
        };
        let selected = select_asset_intel_providers(&[tool_a, tool_b], &[]).unwrap();
        let ids: Vec<String> = selected
            .iter()
            .map(|t| provider_id_for_tool(t).unwrap())
            .collect();
        assert_eq!(
            ids,
            vec![
                "multi-high".to_string(),
                "single-mid".to_string(),
                "multi-low".to_string(),
            ]
        );
    }

    #[test]
    fn provider_descriptors_from_tools_unpacks_multi_provider_tool() {
        let tool = multi_provider_tool(
            "multi",
            &[
                ("multi-a", &["subsidiaries"], true, 100),
                ("multi-b", &["domains"], false, 50),
            ],
        );
        let descriptors = provider_descriptors_from_tools(&[tool]);
        assert_eq!(descriptors.len(), 2);
        let ids: Vec<&str> = descriptors.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"multi-a"));
        assert!(ids.contains(&"multi-b"));
    }

    #[test]
    fn expand_provider_tools_clones_each_provider_into_virtual_tool() {
        let tool = multi_provider_tool(
            "shared",
            &[
                ("shared", &["subsidiaries"], true, 100),
                ("shared-alt", &["subsidiaries"], false, 50),
            ],
        );
        let expanded = expand_provider_tools(&[tool]);
        assert_eq!(expanded.len(), 2);
        assert_eq!(provider_id_for_tool(&expanded[0]).unwrap(), "shared");
        assert_eq!(provider_id_for_tool(&expanded[1]).unwrap(), "shared-alt");
        assert_eq!(expanded[0].executable, "shared/bin");
        assert_eq!(expanded[1].executable, "shared/bin");
        assert!(
            expanded[0].asset_intel_providers.is_none(),
            "virtual tool must not carry providers vec"
        );
        assert!(
            expanded[1].asset_intel_providers.is_none(),
            "virtual tool must not carry providers vec"
        );
    }

    #[test]
    fn expand_provider_tools_passes_single_asset_intel_tool_through_unchanged() {
        let tools = two_phase_fixture_tools();
        let expanded = expand_provider_tools(&tools);
        assert_eq!(
            expanded
                .iter()
                .map(|t| provider_id_for_tool(t).unwrap())
                .collect::<Vec<_>>(),
            vec![
                "enscan-go".to_string(),
                "0.zone".to_string(),
                "legacy".to_string(),
            ],
            "single-provider tools must be cloned 1:1 in scan order"
        );
    }

    #[test]
    fn expand_provider_tools_skips_disabled_providers() {
        let mut tool = multi_provider_tool(
            "shared",
            &[
                ("off", &["subsidiaries"], true, 1),
                ("on", &["subsidiaries"], true, 1),
            ],
        );
        // Mark the first provider disabled so the helper exercises the enabled filter.
        if let Some(providers) = tool.asset_intel_providers.as_mut() {
            providers[0].enabled = false;
        }
        let expanded = expand_provider_tools(&[tool]);
        assert_eq!(expanded.len(), 1);
        assert_eq!(provider_id_for_tool(&expanded[0]).unwrap(), "on");
    }

    #[test]
    fn fixture_discovery_auto_defaults_skip_tyc_until_upstream_is_stable() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let toolsconfig_dir = std::path::PathBuf::from(manifest_dir)
            .join("..")
            .join("..")
            .join("..")
            .join("resources")
            .join("toolsconfig");
        if !toolsconfig_dir.exists() {
            eprintln!(
                "fixture skipped: toolsconfig dir not found at {}",
                toolsconfig_dir.display()
            );
            return;
        }
        let scan = golish_pentest::scan_toolsconfig(&toolsconfig_dir);
        assert!(
            scan.success,
            "toolsconfig scan failed: {:?}",
            scan.error.as_deref()
        );

        let selected = select_subsidiary_providers(&scan.tools, &[]).unwrap();

        assert_eq!(
            selected
                .iter()
                .map(|tool| provider_id_for_tool(tool).unwrap())
                .collect::<Vec<_>>(),
            vec![
                "enscan-go".to_string(),
                "enscan-go-kc-discovery".to_string(),
                "enscan-go-rb-discovery".to_string(),
            ],
            "default discovery should skip TYC while ENScan_GO v2.0.5 TYC discovery is unstable"
        );
    }

    #[test]
    fn fixture_enrichment_profile_fields_cover_observed_provider_keys() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let toolsconfig_dir = std::path::PathBuf::from(manifest_dir)
            .join("..")
            .join("..")
            .join("..")
            .join("resources")
            .join("toolsconfig");
        if !toolsconfig_dir.exists() {
            eprintln!(
                "fixture skipped: toolsconfig dir not found at {}",
                toolsconfig_dir.display()
            );
            return;
        }
        let scan = golish_pentest::scan_toolsconfig(&toolsconfig_dir);
        assert!(
            scan.success,
            "toolsconfig scan failed: {:?}",
            scan.error.as_deref()
        );
        fn has_rule(
            asset: &golish_pentest::models::AssetIntelToolConfig,
            path: &str,
            source: &str,
            target: &str,
            kind: golish_pentest::models::AssetIntelProfileFieldTarget,
        ) -> bool {
            asset.normalize.profile_fields.iter().any(|rule| {
                rule.path == path
                    && rule.target_field == target
                    && rule.target_kind == kind
                    && matches!(
                        &rule.source_field,
                        golish_pentest::models::AssetIntelFieldRef::Field(field) if field == source
                    )
            })
        }

        let expanded = expand_provider_tools(&scan.tools);
        let zone = expanded
            .iter()
            .find(|tool| provider_id_for_tool(tool).as_deref() == Some("0.zone"))
            .and_then(|tool| tool.asset_intel.as_ref())
            .expect("0.zone provider fixture");
        let enscan = expanded
            .iter()
            .find(|tool| provider_id_for_tool(tool).as_deref() == Some("enscan-go-enrichment"))
            .and_then(|tool| tool.asset_intel.as_ref())
            .expect("ENScan enrichment provider fixture");

        assert!(
            has_rule(
                zone,
                "$..data[*]",
                "ip",
                "ip_ranges",
                golish_pentest::models::AssetIntelProfileFieldTarget::Scalar
            ),
            "0.zone site.ip should hydrate organization ip_ranges"
        );
        assert!(
            has_rule(
                zone,
                "$..data[*]",
                "asn",
                "asns",
                golish_pentest::models::AssetIntelProfileFieldTarget::Scalar
            ),
            "0.zone site.asn should hydrate organization asns"
        );
        assert!(
            has_rule(
                zone,
                "$..data[*]",
                "msg.code",
                "credit_code",
                golish_pentest::models::AssetIntelProfileFieldTarget::Scalar
            ),
            "0.zone org.msg.code should hydrate credit_code"
        );
        assert!(
            has_rule(
                enscan,
                "$..enterprise_info[*]",
                "scope",
                "business_scope",
                golish_pentest::models::AssetIntelProfileFieldTarget::Intel
            ),
            "ENScan enterprise scope should be preserved in intel"
        );
        assert!(
            has_rule(
                enscan,
                "$..icp[*]",
                "icp",
                "icp_records",
                golish_pentest::models::AssetIntelProfileFieldTarget::Intel
            ),
            "ENScan ICP license number should be preserved in intel"
        );

        let credit_rule = zone
            .normalize
            .profile_fields
            .iter()
            .find(|rule| {
                rule.target_field == "credit_code"
                    && matches!(
                        &rule.source_field,
                        golish_pentest::models::AssetIntelFieldRef::Field(field) if field == "msg.code"
                    )
            })
            .expect("0.zone msg.code -> credit_code rule must exist");
        assert!(
            credit_rule.when.iter().any(|clause| {
                clause.field == "name_cn"
                    && matches!(
                        clause.op,
                        golish_pentest::models::AssetIntelNormalizeFilterOp::Exists
                    )
            }),
            "0.zone msg.code -> credit_code must require name_cn presence to avoid pulling \
             apk/site/domain msg.code values into the master organization profile"
        );

        for target_kind in [
            golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
            golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
        ] {
            for rule in zone.normalize.profile_fields.iter() {
                if rule.target_kind != target_kind {
                    continue;
                }
                if !matches!(
                    &rule.source_field,
                    golish_pentest::models::AssetIntelFieldRef::Field(field)
                        if matches!(
                            field.as_str(),
                            "msg.industry"
                            | "msg.legal_person"
                            | "msg.reg_address"
                            | "msg.reg_time"
                            | "msg.capital"
                            | "msg.business"
                            | "msg.email[0]"
                            | "msg.contact_number"
                            | "msg.website[0]"
                        )
                ) {
                    continue;
                }
                assert!(
                    rule.when.iter().any(|clause| {
                        clause.field == "name_cn"
                            && matches!(
                                clause.op,
                                golish_pentest::models::AssetIntelNormalizeFilterOp::Exists
                            )
                    }),
                    "0.zone {:?} -> {} rule must require name_cn presence (org-only field), \
                     otherwise apk/site/domain records can pollute the master record",
                    rule.source_field,
                    rule.target_field
                );
            }
        }

        assert!(
            !zone
                .normalize
                .profile_fields
                .iter()
                .any(|rule| rule.target_field == "certificates"),
            "0.zone must not map ssl_certificate (a static-asset URL) into organization \
             certificates; revisit when we add a real cert subject extractor"
        );

        let apk_rule = zone
            .normalize
            .profile_fields
            .iter()
            .find(|rule| {
                rule.target_field == "mobile_apps"
                    && matches!(
                        &rule.source_field,
                        golish_pentest::models::AssetIntelFieldRef::FirstOf(items)
                            if items.iter().any(|s| s == "msg.app_url")
                    )
            })
            .expect("0.zone apk -> mobile_apps rule must exist");
        if let golish_pentest::models::AssetIntelFieldRef::FirstOf(items) = &apk_rule.source_field {
            assert!(
                !items.iter().any(|s| s == "title"),
                "0.zone apk -> mobile_apps must NOT fall back to `title` \
                 (网页 SEO 标题被误塞进 business systems 是上轮发现的 bug)"
            );
        }
    }

    #[test]
    fn select_enrichment_providers_excludes_subsidiaries_capable_tools() {
        let tools = two_phase_fixture_tools();
        let selected = select_enrichment_providers(&tools, &[]).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|t| provider_id_for_tool(t).unwrap())
                .collect::<Vec<_>>(),
            vec!["0.zone".to_string()],
            "0.zone is the only auto-default non-subsidiaries provider"
        );
    }

    #[test]
    fn enrichment_config_disables_candidate_queue_writes() {
        let config = enrichment_hydrate_config(AssetIntelHydrateConfig {
            min_ownership_percent: Some("35".into()),
            depth: Some("2".into()),
            include_branches: Some(true),
            create_candidates: Some(true),
        });

        assert_eq!(config.min_ownership_percent.as_deref(), Some("35"));
        assert_eq!(config.depth.as_deref(), Some("2"));
        assert_eq!(config.include_branches, Some(true));
        assert_eq!(config.create_candidates, Some(false));
    }

    #[test]
    fn enrich_organization_config_disables_candidate_queue_writes() {
        let args = AssetIntelEnrichOrganizationArgs {
            organization_id: Uuid::new_v4().to_string(),
            provider_ids: Vec::new(),
            config: AssetIntelHydrateConfig {
                min_ownership_percent: Some("35".into()),
                depth: Some("2".into()),
                include_branches: Some(true),
                create_candidates: Some(true),
            },
        };

        let config = enrichment_hydrate_config_for_organization(&args);

        assert_eq!(config.min_ownership_percent.as_deref(), Some("35"));
        assert_eq!(config.depth.as_deref(), Some("2"));
        assert_eq!(config.include_branches, Some(true));
        assert_eq!(config.create_candidates, Some(false));
    }

    #[test]
    fn select_subsidiary_providers_rejects_explicit_request_for_enrichment_tool() {
        let tools = two_phase_fixture_tools();
        let err = select_subsidiary_providers(&tools, &["0.zone".to_string()])
            .expect_err("requesting 0.zone for discovery must reject");
        let msg = format!("{err}");
        assert!(
            msg.contains("subsidiaries") && msg.contains("0.zone"),
            "error should mention both the missing capability and the offending provider, got: {msg}"
        );
    }

    #[test]
    fn select_enrichment_providers_rejects_explicit_request_for_subsidiaries_tool() {
        let tools = two_phase_fixture_tools();
        let err = select_enrichment_providers(&tools, &["enscan-go".to_string()])
            .expect_err("requesting enscan-go for enrichment must reject");
        let msg = format!("{err}");
        assert!(
            msg.contains("discovery") && msg.contains("enscan-go"),
            "error should direct caller to hydrate_subsidiaries, got: {msg}"
        );
    }

    #[test]
    fn provider_has_subsidiaries_is_case_insensitive() {
        let tool = ToolConfig {
            id: "casing".into(),
            name: "casing".into(),
            asset_intel: Some(golish_pentest::models::AssetIntelToolConfig {
                enabled: true,
                provider_id: "casing".into(),
                display_name: "casing".into(),
                capabilities: vec!["Subsidiaries".into()],
                requires_integration: None,
                auto: golish_pentest::models::AssetIntelAutoConfig {
                    default: true,
                    priority: 1,
                },
                runtime: fake_runtime(),
                normalize: golish_pentest::models::AssetIntelNormalizeConfig::default(),
                discovery: golish_pentest::models::AssetIntelDiscoveryConfig::default(),
                lookup: None,
            }),
            ..Default::default()
        };
        assert!(
            provider_has_subsidiaries(&tool),
            "capability matching must be case-insensitive so JSON authors don't get bit"
        );
    }

    #[test]
    fn normalize_when_filter_drops_low_ownership_invest_rows() {
        let mut normalize = fake_normalize_config();
        // The org rule covers `$..invest[*]` already; layer a numeric filter
        // that only keeps rows with `scale >= 51`. Anything below should drop
        // out of the candidate pool entirely.
        normalize.organization[0].when = vec![golish_pentest::models::AssetIntelNormalizeFilter {
            field: "scale".into(),
            op: golish_pentest::models::AssetIntelNormalizeFilterOp::Gte,
            value: "51".into(),
        }];
        let raw = serde_json::json!({
            "payload": {
                "invest": [
                    { "name": "全资子公司", "scale": "100" },
                    { "name": "少数股权",   "scale": "5"   },
                    { "name": "缺字段公司"                  },
                ]
            }
        });

        let (candidates, _profile) =
            normalize_json_with_descriptor("filter-provider", "run-filter", 99, &normalize, &raw);

        assert_eq!(
            candidates
                .organizations
                .iter()
                .map(|c| c.label.as_str())
                .collect::<Vec<_>>(),
            vec!["全资子公司"],
            "only rows passing scale>=51 should remain"
        );
    }

    #[test]
    fn normalize_when_filter_contains_op_keeps_matching_rows() {
        let mut normalize = fake_normalize_config();
        normalize.organization[0].when = vec![golish_pentest::models::AssetIntelNormalizeFilter {
            field: "entity_type".into(),
            op: golish_pentest::models::AssetIntelNormalizeFilterOp::Contains,
            value: "公司".into(),
        }];
        let raw = serde_json::json!({
            "data": {
                "invest": [
                    { "name": "测试有限公司", "entity_type": "有限责任公司" },
                    { "name": "个体张三",      "entity_type": "个体工商户"   },
                ]
            }
        });

        let (candidates, _profile) =
            normalize_json_with_descriptor("filter-provider", "run-contains", 1, &normalize, &raw);

        assert_eq!(candidates.organizations.len(), 1);
        assert_eq!(candidates.organizations[0].label, "测试有限公司");
    }

    #[test]
    fn normalize_when_filter_exists_drops_empty_fields() {
        let mut normalize = fake_normalize_config();
        normalize.organization[0].when = vec![golish_pentest::models::AssetIntelNormalizeFilter {
            field: "pid".into(),
            op: golish_pentest::models::AssetIntelNormalizeFilterOp::Exists,
            value: String::new(),
        }];
        let raw = serde_json::json!({
            "data": {
                "invest": [
                    { "name": "已知 pid", "pid": "abc" },
                    { "name": "缺 pid"                 },
                    { "name": "空 pid",   "pid": ""    },
                ]
            }
        });

        let (candidates, _profile) =
            normalize_json_with_descriptor("filter-provider", "run-exists", 1, &normalize, &raw);

        assert_eq!(candidates.organizations.len(), 1);
        assert_eq!(candidates.organizations[0].label, "已知 pid");
    }

    #[test]
    fn extract_profile_field_entries_scalar_intel_contact_buckets() {
        let rules = vec![
            golish_pentest::models::AssetIntelProfileFieldRule {
                path: "$..enterprise_info[*]".into(),
                source_field: golish_pentest::models::AssetIntelFieldRef::Field("reg_code".into()),
                target_field: "credit_code".into(),
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                transform: golish_pentest::models::AssetIntelProfileFieldTransform::None,
                when: vec![],
            },
            golish_pentest::models::AssetIntelProfileFieldRule {
                path: "$..enterprise_info[*]".into(),
                source_field: golish_pentest::models::AssetIntelFieldRef::Field("legal".into()),
                target_field: "legal_representative".into(),
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                transform: golish_pentest::models::AssetIntelProfileFieldTransform::Trim,
                when: vec![],
            },
            golish_pentest::models::AssetIntelProfileFieldRule {
                path: "$..enterprise_info[*]".into(),
                source_field: golish_pentest::models::AssetIntelFieldRef::Field("email".into()),
                target_field: "email".into(),
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                transform: golish_pentest::models::AssetIntelProfileFieldTransform::Lower,
                when: vec![],
            },
            golish_pentest::models::AssetIntelProfileFieldRule {
                path: "$..enterprise_info[*]".into(),
                source_field: golish_pentest::models::AssetIntelFieldRef::Field("phone".into()),
                target_field: "phone".into(),
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                transform: golish_pentest::models::AssetIntelProfileFieldTransform::None,
                when: vec![],
            },
        ];
        let raw = serde_json::json!({
            "payload": {
                "enterprise_info": [
                    {
                        "name": "小米科技",
                        "reg_code": "91110108551385082Q",
                        "legal": "  雷军  ",
                        "email": "Press@MI.com",
                        "phone": "010-12345678"
                    }
                ]
            }
        });

        let entries = extract_profile_field_entries(&rules, &raw);

        assert_eq!(entries.len(), 4);
        let by_field: HashMap<_, _> = entries
            .iter()
            .map(|e| (e.target_field.as_str(), e.value.as_str()))
            .collect();
        assert_eq!(by_field["credit_code"], "91110108551385082Q");
        assert_eq!(by_field["legal_representative"], "雷军"); // trim
        assert_eq!(by_field["email"], "press@mi.com"); // lower
        assert_eq!(by_field["phone"], "010-12345678");
    }

    #[test]
    fn extract_profile_field_entries_when_filter_drops_placeholder_values() {
        // ENScan AQC returns "-" (single dash) as a placeholder for missing
        // email / phone. Without a `when` filter that placeholder would land
        // in organizations.intel.contacts.email and pollute the master record.
        let rules = vec![
            golish_pentest::models::AssetIntelProfileFieldRule {
                path: "$..enterprise_info[*]".into(),
                source_field: golish_pentest::models::AssetIntelFieldRef::Field("email".into()),
                target_field: "email".into(),
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                transform: golish_pentest::models::AssetIntelProfileFieldTransform::Lower,
                when: vec![golish_pentest::models::AssetIntelNormalizeFilter {
                    field: "email".into(),
                    op: golish_pentest::models::AssetIntelNormalizeFilterOp::Ne,
                    value: "-".into(),
                }],
            },
            golish_pentest::models::AssetIntelProfileFieldRule {
                path: "$..enterprise_info[*]".into(),
                source_field: golish_pentest::models::AssetIntelFieldRef::Field("phone".into()),
                target_field: "phone".into(),
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                transform: golish_pentest::models::AssetIntelProfileFieldTransform::Trim,
                when: vec![golish_pentest::models::AssetIntelNormalizeFilter {
                    field: "phone".into(),
                    op: golish_pentest::models::AssetIntelNormalizeFilterOp::Ne,
                    value: "-".into(),
                }],
            },
        ];
        let raw = serde_json::json!({
            "enterprise_info": [
                {
                    // dash placeholders — both must drop out
                    "email": "-",
                    "phone": "-"
                },
                {
                    // real values — must pass through
                    "email": "Press@MI.com",
                    "phone": "010-12345678"
                }
            ]
        });

        let entries = extract_profile_field_entries(&rules, &raw);

        assert_eq!(entries.len(), 2, "only the real-value row survives");
        assert_eq!(entries[0].target_field, "email");
        assert_eq!(entries[0].value, "press@mi.com");
        assert_eq!(entries[1].target_field, "phone");
        assert_eq!(entries[1].value, "010-12345678");
    }

    #[test]
    fn build_profile_patch_first_wins_for_scalar_intel_contact_dedupes() {
        let entries = vec![
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "credit_code".into(),
                value: "AAA".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "credit_code".into(),
                value: "BBB".into(), // duplicate from another provider — must NOT overwrite
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "industry".into(),
                value: "互联网".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "legal_representative".into(),
                value: "雷军".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                target_field: "email".into(),
                value: "a@example.com".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                target_field: "email".into(),
                value: "A@example.com".into(), // case-only diff → must dedupe
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Contact,
                target_field: "email".into(),
                value: "b@example.com".into(),
            },
        ];
        let existing_intel = serde_json::json!({
            "contacts": { "email": ["preexisting@example.com"] },
            "engagement": { "mode": "discover_assets" }
        });

        let patch = build_profile_patch_from_entries(&existing_intel, &entries)
            .expect("patch build ok")
            .expect("patch is Some when entries present");

        assert_eq!(patch.credit_code.as_deref(), Some("AAA"));
        assert_eq!(patch.industry.as_deref(), Some("互联网"));
        let intel = patch.intel.expect("intel patched");
        assert_eq!(
            intel["legal_representative"],
            serde_json::Value::String("雷军".into())
        );
        let emails = intel["contacts"]["email"].as_array().expect("email array");
        let strs: Vec<&str> = emails.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(
            strs,
            vec!["preexisting@example.com", "a@example.com", "b@example.com"]
        );
        // engagement metadata must survive
        assert_eq!(
            intel["engagement"]["mode"],
            serde_json::Value::String("discover_assets".into())
        );
    }

    #[test]
    fn build_profile_patch_dedupes_multi_value_intel_fields() {
        let entries = vec![
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "icp_records".into(),
                value: "粤ICP备06118290号-2".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "icp_records".into(),
                value: "粤ICP备06118290号-2".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "icp_records".into(),
                value: "粤ICP备06118290号-16".into(),
            },
        ];
        let existing_intel = serde_json::json!({
            "icp_records": ["粤ICP备06118290号-1"]
        });

        let patch = build_profile_patch_from_entries(&existing_intel, &entries)
            .expect("patch build ok")
            .expect("patch is Some when entries present");

        assert_eq!(
            patch.intel.expect("intel patched")["icp_records"],
            serde_json::json!([
                "粤ICP备06118290号-1",
                "粤ICP备06118290号-2",
                "粤ICP备06118290号-16"
            ])
        );
    }

    #[test]
    fn build_profile_patch_dedupes_app_intel_array_fields() {
        let entries = vec![
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "mobile_apps".into(),
                value: "小米实况麻将".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "mobile_apps".into(),
                value: "小米实况麻将".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "mini_programs".into(),
                value: "小米商城".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Intel,
                target_field: "app_domains".into(),
                value: "https://com.dfwe".into(),
            },
        ];

        let patch = build_profile_patch_from_entries(&serde_json::json!({}), &entries)
            .expect("patch build ok")
            .expect("app intel entries should produce a patch");
        let intel = patch.intel.expect("intel patched");

        assert_eq!(intel["mobile_apps"], serde_json::json!(["小米实况麻将"]));
        assert_eq!(intel["mini_programs"], serde_json::json!(["小米商城"]));
        assert_eq!(
            intel["app_domains"],
            serde_json::json!(["https://com.dfwe"])
        );
    }

    #[test]
    fn build_profile_patch_writes_visible_profile_array_fields() {
        let entries = vec![
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "domains".into(),
                value: "example.com".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "domains".into(),
                value: "EXAMPLE.com".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "email_domains".into(),
                value: "example.com".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "business_systems".into(),
                value: "Example App".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "social_accounts".into(),
                value: "wechat:example".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "contacts".into(),
                value: "ir@example.com".into(),
            },
        ];

        let patch = build_profile_patch_from_entries(&serde_json::json!({}), &entries)
            .expect("patch build ok")
            .expect("patch is Some when profile fields are present");

        assert_eq!(patch.domains, Some(serde_json::json!(["example.com"])));
        assert_eq!(
            patch.email_domains,
            Some(serde_json::json!(["example.com"]))
        );
        assert_eq!(
            patch.business_systems,
            Some(serde_json::json!(["Example App"]))
        );
        assert_eq!(
            patch.social_accounts,
            Some(serde_json::json!(["wechat:example"]))
        );
        assert_eq!(patch.contacts, Some(serde_json::json!(["ir@example.com"])));
    }

    #[test]
    fn extract_profile_fields_normalizes_asn_values() {
        let rules = vec![golish_pentest::models::AssetIntelProfileFieldRule {
            path: "$..data[*]".into(),
            source_field: golish_pentest::models::AssetIntelFieldRef::Field("asn".into()),
            target_field: "asns".into(),
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            transform: golish_pentest::models::AssetIntelProfileFieldTransform::Asn,
            when: vec![],
        }];
        let raw = serde_json::json!({
            "data": [
                { "asn": 4134 },
                { "asn": " as37963 " },
                { "asn": "not-an-asn" }
            ]
        });

        let entries = extract_profile_field_entries(&rules, &raw);
        let patch = build_profile_patch_from_entries(&serde_json::json!({}), &entries)
            .expect("patch build ok")
            .expect("asn entries should produce a patch");

        assert_eq!(patch.asns, Some(serde_json::json!(["AS4134", "AS37963"])));
    }

    #[test]
    fn team_cymru_asn_lookup_builds_profile_entries_from_public_ips() {
        let entries = vec![
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "ip_ranges".into(),
                value: "183.62.123.10".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "ip_ranges".into(),
                value: "182.92.121.121".into(),
            },
            ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "ip_ranges".into(),
                value: "10.0.0.1".into(),
            },
        ];
        let response = "\
AS      | IP               | BGP Prefix          | CC | Registry | Allocated  | AS Name
4134    | 183.62.123.10    | 183.56.0.0/13       | CN | apnic    | 2009-09-29 | CHINANET-BACKBONE
37963   | 182.92.121.121   | 182.92.0.0/16       | CN | apnic    | 2013-08-16 | ALIBABA-CN-NET
";

        let ips = collect_public_ips_for_asn_lookup(&entries);
        let mappings = parse_team_cymru_asn_response(response);
        let derived = profile_asn_entries_from_mappings(&mappings);
        let patch = build_profile_patch_from_entries(&serde_json::json!({}), &derived)
            .expect("patch build ok")
            .expect("derived ASN entries should produce a patch");

        assert_eq!(
            ips.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec!["183.62.123.10", "182.92.121.121"]
        );
        assert_eq!(patch.asns, Some(serde_json::json!(["AS4134", "AS37963"])));
    }

    #[test]
    fn build_profile_patch_returns_none_for_empty_or_blank_entries() {
        let entries = vec![ProfileFieldEntry {
            target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
            target_field: "credit_code".into(),
            value: "   ".into(),
        }];
        let intel = serde_json::json!({});
        let patch = build_profile_patch_from_entries(&intel, &entries).unwrap();
        assert!(
            patch.is_none(),
            "all-blank entries should not produce a patch"
        );
    }

    #[test]
    fn extract_lookup_matches_maps_enterprise_info_into_disambiguation_rows() {
        let config = golish_pentest::models::AssetIntelLookupConfig {
            enabled: true,
            skill_id: "company-lookup-json".into(),
            timeout_secs: 60,
            normalize: golish_pentest::models::AssetIntelLookupNormalize {
                path: "$..enterprise_info[*]".into(),
                name: golish_pentest::models::AssetIntelFieldRef::Field("name".into()),
                credit_code: Some(golish_pentest::models::AssetIntelFieldRef::Field(
                    "reg_code".into(),
                )),
                industry: Some(golish_pentest::models::AssetIntelFieldRef::Field(
                    "industry".into(),
                )),
                legal_representative: Some(golish_pentest::models::AssetIntelFieldRef::FirstOf(
                    vec!["legal_person".into(), "legal".into()],
                )),
                address: Some(golish_pentest::models::AssetIntelFieldRef::FirstOf(vec![
                    "reg_address".into(),
                    "addr".into(),
                ])),
                registered_at: Some(golish_pentest::models::AssetIntelFieldRef::Field(
                    "reg_date".into(),
                )),
                score: None,
                default_confidence: 0.68,
            },
        };
        let raw = serde_json::json!({
            "payload": {
                "enterprise_info": [
                    {
                        "name": "小米科技有限责任公司",
                        "reg_code": "91110108551385082Q",
                        "industry": "互联网",
                        "legal_person": "雷军",
                        "reg_address": "北京市海淀区清河中街68号",
                        "reg_date": "2010-03-03"
                    },
                    {
                        "name": "小米通讯技术有限公司",
                        "reg_code": "91440300325990618B",
                        "legal": "回退法人字段",
                        "addr": "回退地址字段"
                    }
                ]
            }
        });

        let matches = extract_lookup_matches("enscan-go", &config, &raw);

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].provider_id, "enscan-go");
        assert_eq!(matches[0].name, "小米科技有限责任公司");
        assert_eq!(
            matches[0].credit_code.as_deref(),
            Some("91110108551385082Q")
        );
        assert_eq!(matches[0].industry.as_deref(), Some("互联网"));
        assert_eq!(matches[0].legal_representative.as_deref(), Some("雷军"));
        assert_eq!(
            matches[0].address.as_deref(),
            Some("北京市海淀区清河中街68号")
        );
        assert_eq!(matches[0].registered_at.as_deref(), Some("2010-03-03"));
        assert!((matches[0].confidence - 0.68).abs() < f64::EPSILON);

        assert_eq!(matches[1].name, "小米通讯技术有限公司");
        assert_eq!(
            matches[1].legal_representative.as_deref(),
            Some("回退法人字段")
        );
        assert_eq!(matches[1].address.as_deref(), Some("回退地址字段"));
        assert!(
            matches[1].industry.is_none(),
            "missing field should stay None"
        );
        assert!(matches[1].registered_at.is_none());
    }

    #[test]
    fn dedupe_lookup_matches_prefers_credit_code_for_uniqueness() {
        let m1 = LookupCompanyMatch {
            provider_id: "enscan-go".into(),
            name: "小米科技有限责任公司".into(),
            credit_code: Some("91110108551385082Q".into()),
            industry: None,
            legal_representative: None,
            address: None,
            registered_at: None,
            confidence: 0.68,
            evidence: serde_json::json!({}),
        };
        let m2 = LookupCompanyMatch {
            provider_id: "another".into(),
            // Different display name but same credit code → must dedupe.
            name: "Xiaomi Inc".into(),
            credit_code: Some("91110108551385082q".into()), // case differs
            industry: None,
            legal_representative: None,
            address: None,
            registered_at: None,
            confidence: 0.5,
            evidence: serde_json::json!({}),
        };
        let m3 = LookupCompanyMatch {
            provider_id: "enscan-go".into(),
            name: "Acme Inc".into(),
            credit_code: None,
            industry: None,
            legal_representative: None,
            address: None,
            registered_at: None,
            confidence: 0.42,
            evidence: serde_json::json!({}),
        };
        let m4 = LookupCompanyMatch {
            provider_id: "another".into(),
            name: "  acme inc  ".into(), // case + whitespace only diff → must dedupe
            credit_code: None,
            industry: None,
            legal_representative: None,
            address: None,
            registered_at: None,
            confidence: 0.3,
            evidence: serde_json::json!({}),
        };

        let deduped = dedupe_lookup_matches(vec![m1.clone(), m2, m3.clone(), m4]);

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].provider_id, "enscan-go");
        assert_eq!(deduped[0].name, m1.name);
        assert_eq!(deduped[1].name, "Acme Inc");
    }

    #[test]
    fn merge_candidates_dedupes_same_value_across_providers() {
        let mut merged = normalize_provider_records(
            "first-provider",
            "run-1",
            1,
            vec![AssetIntelProviderRecord {
                kind: OrganizationCandidateKind::Target,
                label: "api.example.com".into(),
                value: "api.example.com".into(),
                confidence: 0.8,
                evidence: serde_json::json!({"provider": "enscan"}),
            }],
        );
        let zone = normalize_provider_records(
            "second-provider",
            "run-1",
            1,
            vec![AssetIntelProviderRecord {
                kind: OrganizationCandidateKind::Target,
                label: "duplicate".into(),
                value: "API.EXAMPLE.COM".into(),
                confidence: 0.7,
                evidence: serde_json::json!({"provider": "zone"}),
            }],
        );

        merge_candidates(&mut merged, zone);

        assert_eq!(merged.targets.len(), 1);
        assert_eq!(merged.targets[0].source, "first-provider");
        assert_eq!(
            merged.targets[0]
                .evidence
                .get("sources")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
    }
}
