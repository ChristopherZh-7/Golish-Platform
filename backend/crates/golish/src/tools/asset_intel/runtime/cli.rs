//! CLI-JSON provider runner (stdout + artifact scanning). Moved verbatim.

use super::super::*;

pub(crate) fn asset_intel_provider_output_dir(
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
pub(crate) const PROVIDER_PROGRESS_LINE_LIMIT: usize = 512;

/// Polling interval for the `out_dir` artifact watcher (cli_json runtime).
///
/// The frontend's perceived "first candidate in N seconds" is bounded by
/// this interval. Tuned to a sweet spot: small enough to feel live (<1s),
/// large enough to avoid hot-looping `read_dir` during long scans.
pub(crate) const ARTIFACT_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Shared, normalize-and-emit-once accumulator used by the cli_json runner.
///
/// Keeping the accumulator + the cancel flag in a single Arc-wrapped struct
/// lets us hand a cheap clone to every background task (stdout reader,
/// stderr reader, artifact watcher) without juggling individual Arcs.
#[derive(Debug)]
pub(crate) struct CliJsonStreamShared {
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

pub(crate) fn truncate_progress_line(raw: &str) -> String {
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
pub(crate) async fn handle_stdout_line(
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
pub(crate) async fn scan_new_artifacts(
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
            let profile_entries = std::mem::take(&mut *shared.profile_entries.lock().await);
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
                    "reason": "wait_failed",
                    "error": err.to_string(),
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
            let candidate_count = candidates.organizations.len() + candidates.targets.len();
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
            }),
            profile_entries,
        );
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
    finish_provider_run(
        sink,
        run_id,
        status,
        total,
        candidates,
        serde_json::json!({
            "provider": provider_id,
            "runId": run_id,
            "state": if total == 0 { "checked_empty" } else { "completed" },
            "candidateCount": total,
            "outDir": out_dir,
        }),
        profile_entries,
    )
}

pub(crate) fn collect_json_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), GolishError> {
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

pub(crate) fn now_millis() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}
