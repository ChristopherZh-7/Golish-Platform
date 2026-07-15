//! CLI-JSON provider runner (stdout + artifact scanning).
//!
//! The shared stream accumulator and artifact-scanning helpers live in the
//! [`stream`] sibling module; this file holds the top-level runner.

use super::super::*;
use crate::organization_recon::artifacts::{
    decode_utf8_clean, write_json_manifest, write_raw_bytes,
};
use crate::organization_recon::{ReconTaskError, ReconTaskManifest, ReconTaskStatus};

mod stream;
pub(crate) use stream::*;

pub(crate) fn cli_json_empty_result_failure_pattern(
    stdout_raw: &[u8],
    stderr_raw: &[u8],
    patterns: &[String],
) -> Option<String> {
    let stdout = String::from_utf8_lossy(stdout_raw);
    let stderr = String::from_utf8_lossy(stderr_raw);
    patterns.iter().find_map(|pattern| {
        let pattern = pattern.trim();
        (!pattern.is_empty() && (stdout.contains(pattern) || stderr.contains(pattern)))
            .then(|| pattern.to_string())
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_cli_json_provider(
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
    let (provider_id, display_name) = provider_identity(tool, asset);
    let golish_pentest::models::AssetIntelRuntimeConfig::CliJson {
        skill_id,
        timeout_secs,
        artifact_globs: _,
        arg_bindings,
        empty_result_failure_patterns,
    } = &asset.runtime
    else {
        return Err(GolishError::Validation(format!(
            "tool '{}' is not a cli_json provider",
            tool.id
        )));
    };

    emit_provider_started(
        sink,
        run_id,
        &provider_id,
        display_name,
        AssetIntelProviderRuntimeKind::CliJson,
    );

    let Some(exec) = golish_pentest::resolve_tool_executable(&tool.id, tools, tools_dir) else {
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Unavailable,
            message: format!("tool '{}' executable is unavailable", tool.id),
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
                "reason": "tool_executable_unavailable",
            }),
            Vec::new(),
        );
    };
    let Some(skill) = tool.skills.iter().find(|skill| skill.id == *skill_id) else {
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Unavailable,
            message: format!("asset intel skill '{skill_id}' is not declared"),
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
                "reason": "skill_not_found",
                "skillId": skill_id,
            }),
            Vec::new(),
        );
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
                    "reason": "spawn_failed",
                    "error": err.to_string(),
                }),
                Vec::new(),
            );
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
            let mut reader = tokio::io::BufReader::new(stream);
            loop {
                let mut raw = Vec::new();
                let read = match reader.read_until(b'\n', &mut raw).await {
                    Ok(read) => read,
                    Err(error) => {
                        shared.diagnostics.lock().await.push(ReconTaskError::new(
                            "stdout_read_error",
                            format!("cannot read provider stdout: {error}"),
                        ));
                        break;
                    }
                };
                if read == 0 {
                    break;
                }
                shared.stdout_raw.lock().await.extend_from_slice(&raw);
                let line = match decode_utf8_clean(&raw) {
                    Ok(line) => line,
                    Err(error) => {
                        shared.diagnostics.lock().await.push(error);
                        continue;
                    }
                };
                {
                    let mut buf = shared.progress_buffer.lock().await;
                    buf.push_str(&line);
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
            let mut reader = tokio::io::BufReader::new(stream);
            loop {
                let mut raw = Vec::new();
                let read = match reader.read_until(b'\n', &mut raw).await {
                    Ok(read) => read,
                    Err(error) => {
                        shared.diagnostics.lock().await.push(ReconTaskError::new(
                            "stderr_read_error",
                            format!("cannot read provider stderr: {error}"),
                        ));
                        break;
                    }
                };
                if read == 0 {
                    break;
                }
                shared.stderr_raw.lock().await.extend_from_slice(&raw);
                let line = match decode_utf8_clean(&raw) {
                    Ok(line) => line,
                    Err(error) => {
                        shared.diagnostics.lock().await.push(error);
                        continue;
                    }
                };
                {
                    let mut buf = shared.progress_buffer.lock().await;
                    buf.push_str(&line);
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
                    false,
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
            let candidates = std::mem::take(&mut *shared.candidates.lock().await);
            let profile_entries = std::mem::take(&mut *shared.profile_entries.lock().await);
            let stdout_raw = std::mem::take(&mut *shared.stdout_raw.lock().await);
            let stderr_raw = std::mem::take(&mut *shared.stderr_raw.lock().await);
            let mut diagnostics = std::mem::take(&mut *shared.diagnostics.lock().await);
            diagnostics.push(ReconTaskError::new("wait_failed", err.to_string()));
            let candidate_count = candidates.organizations.len() + candidates.targets.len();
            let record_count = candidate_count + profile_entries.len();
            let manifest_path = persist_cli_artifacts(
                &out_dir,
                run_id,
                &provider_id,
                ReconTaskStatus::Failed,
                None,
                record_count,
                &stdout_raw,
                &stderr_raw,
                diagnostics,
            )?;
            return finish_provider_run(
                sink,
                run_id,
                status,
                candidate_count,
                candidates,
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "failed",
                    "reason": "wait_failed",
                    "error": err.to_string(),
                    "candidateCount": candidate_count,
                    "manifestPath": manifest_path,
                }),
                profile_entries,
            );
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
            let stdout_raw = std::mem::take(&mut *shared.stdout_raw.lock().await);
            let stderr_raw = std::mem::take(&mut *shared.stderr_raw.lock().await);
            let mut diagnostics = std::mem::take(&mut *shared.diagnostics.lock().await);
            diagnostics.push(ReconTaskError::new(
                "timeout",
                format!("command timed out after {}s", timeout.as_secs()),
            ));
            let candidate_count = candidates.organizations.len() + candidates.targets.len();
            let record_count = candidate_count + profile_entries.len();
            let manifest_path = persist_cli_artifacts(
                &out_dir,
                run_id,
                &provider_id,
                ReconTaskStatus::Failed,
                None,
                record_count,
                &stdout_raw,
                &stderr_raw,
                diagnostics,
            )?;
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!("command timed out after {}s", timeout.as_secs()),
            };
            return finish_provider_run(
                sink,
                run_id,
                status,
                candidate_count,
                candidates,
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "failed",
                    "reason": "timeout",
                    "timeoutSecs": timeout.as_secs(),
                    "candidateCount": candidate_count,
                    "manifestPath": manifest_path,
                }),
                profile_entries,
            );
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
        true,
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
    let stdout_raw = std::mem::take(&mut *shared.stdout_raw.lock().await);
    let stderr_raw = std::mem::take(&mut *shared.stderr_raw.lock().await);
    let mut diagnostics = std::mem::take(&mut *shared.diagnostics.lock().await);
    let preview: String = progress_buffer.chars().take(512).collect();
    let candidate_count = candidates.organizations.len() + candidates.targets.len();
    let record_count = candidate_count + profile_entries.len();

    if !exit_status.success() {
        tracing::warn!(
            provider = %provider_id,
            run_id,
            exit_code = exit_status.code(),
            "asset_intel cli_json provider exited unsuccessfully"
        );
        diagnostics.push(ReconTaskError::new(
            "command_failed",
            format!("provider command exited with code {:?}", exit_status.code()),
        ));
        let manifest_path = persist_cli_artifacts(
            &out_dir,
            run_id,
            &provider_id,
            ReconTaskStatus::Failed,
            exit_status.code(),
            record_count,
            &stdout_raw,
            &stderr_raw,
            diagnostics,
        )?;
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Failed,
            message: format!("command failed: {preview}"),
        };
        return finish_provider_run(
            sink,
            run_id,
            status,
            candidate_count,
            candidates,
            serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "state": "failed",
                "reason": "command_failed",
                "exitCode": exit_status.code(),
                "preview": preview,
                "candidateCount": candidate_count,
                "manifestPath": manifest_path,
            }),
            profile_entries,
        );
    }

    if !diagnostics.is_empty() {
        let manifest_path = persist_cli_artifacts(
            &out_dir,
            run_id,
            &provider_id,
            ReconTaskStatus::Failed,
            exit_status.code(),
            record_count,
            &stdout_raw,
            &stderr_raw,
            diagnostics,
        )?;
        let status = AssetIntelProviderRunStatus {
            provider_id: provider_id.clone(),
            status: AssetIntelProviderRunState::Failed,
            message: format!("{provider_id} completed with invalid output"),
        };
        return finish_provider_run(
            sink,
            run_id,
            status,
            candidate_count,
            candidates,
            serde_json::json!({
                "provider": provider_id,
                "runId": run_id,
                "state": "failed",
                "reason": "invalid_output",
                "candidateCount": candidate_count,
                "manifestPath": manifest_path,
            }),
            profile_entries,
        );
    }

    if record_count == 0 {
        if let Some(pattern) = cli_json_empty_result_failure_pattern(
            &stdout_raw,
            &stderr_raw,
            empty_result_failure_patterns,
        ) {
            diagnostics.push(ReconTaskError::new(
                "provider_reported_failure",
                format!("empty provider output matched configured failure pattern: {pattern}"),
            ));
            let manifest_path = persist_cli_artifacts(
                &out_dir,
                run_id,
                &provider_id,
                ReconTaskStatus::Failed,
                exit_status.code(),
                record_count,
                &stdout_raw,
                &stderr_raw,
                diagnostics,
            )?;
            let status = AssetIntelProviderRunStatus {
                provider_id: provider_id.clone(),
                status: AssetIntelProviderRunState::Failed,
                message: format!(
                    "{provider_id} reported an upstream failure despite exit code zero"
                ),
            };
            return finish_provider_run(
                sink,
                run_id,
                status,
                candidate_count,
                candidates,
                serde_json::json!({
                    "provider": provider_id,
                    "runId": run_id,
                    "state": "failed",
                    "reason": "provider_reported_failure",
                    "matchedPattern": pattern,
                    "candidateCount": candidate_count,
                    "manifestPath": manifest_path,
                }),
                profile_entries,
            );
        }
    }

    let state = if record_count == 0 {
        AssetIntelProviderRunState::CheckedEmpty
    } else {
        AssetIntelProviderRunState::Completed
    };
    let manifest_path = persist_cli_artifacts(
        &out_dir,
        run_id,
        &provider_id,
        if record_count == 0 {
            ReconTaskStatus::CheckedEmpty
        } else {
            ReconTaskStatus::Completed
        },
        exit_status.code(),
        record_count,
        &stdout_raw,
        &stderr_raw,
        diagnostics,
    )?;
    tracing::info!(
        provider = %provider_id,
        run_id,
        candidate_count,
        profile_field_count = profile_entries.len(),
        state = ?state,
        "asset_intel cli_json provider completed"
    );
    let status = AssetIntelProviderRunStatus {
        provider_id: provider_id.clone(),
        status: state,
        message: if record_count == 0 {
            format!("{provider_id} completed with no candidates")
        } else {
            format!("{provider_id} normalized {record_count} record(s)")
        },
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
            "outDir": out_dir,
            "manifestPath": manifest_path,
        }),
        profile_entries,
    )
}

#[allow(clippy::too_many_arguments)]
fn persist_cli_artifacts(
    out_dir: &Path,
    run_id: &str,
    provider_id: &str,
    status: ReconTaskStatus,
    exit_code: Option<i32>,
    record_count: usize,
    stdout_raw: &[u8],
    stderr_raw: &[u8],
    errors: Vec<ReconTaskError>,
) -> Result<PathBuf, GolishError> {
    let mut manifest = ReconTaskManifest::new(run_id, provider_id, "enterprise_intel", provider_id);
    manifest.status = status;
    manifest.exit_code = exit_code;
    manifest.record_count = record_count;
    manifest.checked_empty = matches!(manifest.status, ReconTaskStatus::CheckedEmpty);
    manifest.errors = errors;
    manifest.artifacts.push(write_raw_bytes(
        out_dir,
        "raw/stdout.log",
        stdout_raw,
        "stdout",
    )?);
    manifest.artifacts.push(write_raw_bytes(
        out_dir,
        "raw/stderr.log",
        stderr_raw,
        "stderr",
    )?);
    write_json_manifest(out_dir, &manifest)
}
