//! Synchronous lookup-CLI provider runner. Moved out of `mod.rs` verbatim.

use super::super::*;

/// Per-provider output directory used by the lookup runtime, scoped under
/// `<project>/.golish/tool-output/asset-intel-lookup/<runId>/<providerId>`.
/// Keeps lookup artifacts separate from full hydrate runs so cleanup is
/// trivial and there's no risk of mixing canonical vs. discovery output.
pub(crate) fn lookup_provider_output_dir(
    project_root: &Path,
    run_id: &str,
    provider_id: &str,
) -> PathBuf {
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
pub(crate) async fn run_lookup_cli_provider(
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
