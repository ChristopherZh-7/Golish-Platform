use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use golish_app_core::GolishError;
use golish_pentest::models::{InstallInfoExt, ToolConfig};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

use super::artifacts::{
    decode_utf8_clean, write_json_manifest, write_raw_bytes, write_records_jsonl,
};
use super::normalize::{merge_normalized_records, normalize_record_key};
use super::types::{
    NormalizedReconRecord, ReconArtifactRef, ReconEvidenceRef, ReconRecordKind, ReconTaskError,
    ReconTaskManifest, ReconTaskStatus,
};
use crate::targets::Target;

const MAX_ACTIVE_SEEDS: usize = 10;
const ACTIVE_TOOL_INSTALL_TMP: &str = "golish-recon-active-install";
const NMAP_COMMON_PORTS: &str = "80,443,8080,8443,8000,8008,8888,8081,3000,5000,7001,9000,9443";
const NMAP_TIMEOUT_SECS: u64 = 120;

pub(crate) type ActiveLogSender = UnboundedSender<ActiveCollectionLog>;

#[derive(Debug)]
pub(crate) struct ActiveCollectionOutcome {
    pub status: ReconTaskStatus,
    pub record_count: usize,
    pub errors: Vec<ReconTaskError>,
    pub artifacts: Vec<ReconArtifactRef>,
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveCollectionLog {
    pub level: String,
    pub message: String,
    pub task_id: Option<String>,
}

impl ActiveCollectionLog {
    fn info(message: impl Into<String>) -> Self {
        Self {
            level: "info".into(),
            message: message.into(),
            task_id: None,
        }
    }

    fn warn(message: impl Into<String>) -> Self {
        Self {
            level: "warning".into(),
            message: message.into(),
            task_id: None,
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            level: "error".into(),
            message: message.into(),
            task_id: None,
        }
    }

    fn for_task(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }
}

#[derive(Debug, Clone)]
struct ActiveTask {
    tool_id: String,
    seed: String,
    args: Vec<String>,
    timeout_secs: u64,
}

#[derive(Debug, Default)]
struct ActiveScopeSet {
    roots: BTreeSet<String>,
    hosts: BTreeSet<String>,
    urls: BTreeSet<String>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_active_collection(
    scan_tools: &[ToolConfig],
    tools_dir: &Path,
    proxy_url: Option<&str>,
    github_token: Option<&str>,
    run_id: &str,
    active_targets: &[Target],
    active_dir: &Path,
    log_tx: Option<ActiveLogSender>,
) -> Result<ActiveCollectionOutcome, GolishError> {
    let scope = ActiveScopeSet::from_targets(active_targets);
    let scope_bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "runId": run_id,
        "rootDomains": scope.roots,
        "hosts": scope.hosts,
        "urls": scope.urls,
        "targetCount": active_targets.len(),
    }))
    .map_err(|error| GolishError::Internal(format!("serialize active scope failed: {error}")))?;
    let mut artifacts = vec![write_raw_bytes(
        active_dir,
        "raw/active-scope.json",
        &scope_bytes,
        "active_scope",
    )?];

    let tasks = planned_tasks(&scope);
    let mut logs = Vec::new();
    emit_active_log(
        &mut logs,
        &log_tx,
        ActiveCollectionLog::info(format!(
            "active_collection planned {} tool task(s) for {} target(s)",
            tasks.len(),
            active_targets.len()
        )),
    );
    if tasks.is_empty() {
        return Ok(ActiveCollectionOutcome {
            status: ReconTaskStatus::CheckedEmpty,
            record_count: 0,
            errors: Vec::new(),
            artifacts,
        });
    }

    let mut errors = Vec::new();
    let mut records = Vec::new();
    for task in tasks {
        match run_active_task(
            scan_tools,
            tools_dir,
            proxy_url,
            github_token,
            run_id,
            active_dir,
            &scope,
            task,
            log_tx.clone(),
        )
        .await
        {
            Ok(result) => {
                artifacts.extend(result.artifacts);
                errors.extend(result.errors);
                records.extend(result.records);
            }
            Err(error) => {
                let message = format!("active_task_failed: {error}");
                tracing::warn!(run_id = %run_id, error = %error, "organization_recon active task failed");
                emit_active_log(
                    &mut logs,
                    &log_tx,
                    ActiveCollectionLog::error(message.clone()),
                );
                errors.push(ReconTaskError::new("active_task_failed", message));
            }
        }
    }

    let records = merge_normalized_records(records);
    let records_artifact = write_records_jsonl(active_dir, &records)?;
    artifacts.push(records_artifact);

    let status = if records.is_empty() {
        ReconTaskStatus::CheckedEmpty
    } else {
        ReconTaskStatus::Completed
    };

    Ok(ActiveCollectionOutcome {
        status,
        record_count: records.len(),
        errors,
        artifacts,
    })
}

struct ActiveTaskResult {
    records: Vec<NormalizedReconRecord>,
    errors: Vec<ReconTaskError>,
    artifacts: Vec<ReconArtifactRef>,
}

struct ActiveTaskManifestInput<'a> {
    task_dir: &'a Path,
    run_id: &'a str,
    task: &'a ActiveTask,
    status: ReconTaskStatus,
    exit_code: Option<i32>,
    record_count: usize,
    errors: &'a [ReconTaskError],
}

#[allow(clippy::too_many_arguments)]
async fn resolve_or_install_active_tool(
    tool: &ToolConfig,
    scan_tools: &[ToolConfig],
    tools_dir: &Path,
    proxy_url: Option<&str>,
    github_token: Option<&str>,
    task_log_id: &str,
    logs: &mut Vec<ActiveCollectionLog>,
    log_tx: &Option<ActiveLogSender>,
) -> Result<String, ReconTaskError> {
    if let Some(exec) = golish_pentest::resolve_tool_executable(&tool.id, scan_tools, tools_dir) {
        if let Some(exec) = executable_candidate_path(&exec, tools_dir) {
            let exec = exec.display().to_string();
            if active_tool_binary_usable(&tool.id, &exec).await {
                return Ok(exec);
            }
            emit_active_log(
                logs,
                log_tx,
                ActiveCollectionLog::warn(format!(
                    "active_tool_validation_failed: tool={} executable={} is not compatible with expected arguments",
                    tool.id, exec
                ))
                .for_task(task_log_id),
            );
        }
    }

    if let Some(exec) = managed_tool_executable(tool, tools_dir) {
        if active_tool_binary_usable(&tool.id, &exec).await {
            emit_active_log(
                logs,
                log_tx,
                ActiveCollectionLog::info(format!(
                    "active_tool_managed_executable_found: tool={} executable={}",
                    tool.id, exec
                ))
                .for_task(task_log_id),
            );
            return Ok(exec);
        }
    }

    let Some(install) = tool.install.as_ref() else {
        let message = format!(
            "active_tool_install_unavailable: tool={} has no install descriptor",
            tool.id
        );
        emit_active_log(
            logs,
            log_tx,
            ActiveCollectionLog::error(message.clone()).for_task(task_log_id),
        );
        return Err(ReconTaskError::new(
            "active_tool_install_unavailable",
            message,
        ));
    };
    let Some((method, source)) = install.resolve_for_current_platform() else {
        let message = format!(
            "active_tool_install_unavailable: tool={} install method is unsupported on this platform",
            tool.id
        );
        emit_active_log(
            logs,
            log_tx,
            ActiveCollectionLog::error(message.clone()).for_task(task_log_id),
        );
        return Err(ReconTaskError::new(
            "active_tool_install_unavailable",
            message,
        ));
    };
    if method.trim().is_empty() {
        let message = format!(
            "active_tool_install_unavailable: tool={} install method is empty",
            tool.id
        );
        emit_active_log(
            logs,
            log_tx,
            ActiveCollectionLog::error(message.clone()).for_task(task_log_id),
        );
        return Err(ReconTaskError::new(
            "active_tool_install_unavailable",
            message,
        ));
    }

    emit_active_log(
        logs,
        log_tx,
        ActiveCollectionLog::info(format!(
            "active_tool_auto_install_start: tool={} method={} source={}",
            tool.id, method, source
        ))
        .for_task(task_log_id),
    );

    let install_result = match method.as_str() {
        "homebrew" | "github" | "gem" => {
            install_tool_via_tool_manager(
                tool,
                &method,
                &source,
                tools_dir,
                proxy_url,
                github_token,
                task_log_id,
                logs,
                log_tx,
            )
            .await
        }
        "homebrew-cask" => {
            install_runtime_via_tool_manager(
                tool,
                &format!("brew-cask:{source}"),
                proxy_url,
                task_log_id,
                logs,
                log_tx,
            )
            .await
        }
        "pip" => Err("pip auto install from active collection is not wired yet".into()),
        _ => Err(format!("unsupported install method: {method}")),
    };

    if let Err(error) = install_result {
        let message = format!(
            "active_tool_auto_install_failed: tool={} method={} source={} error={}",
            tool.id, method, source, error
        );
        emit_active_log(
            logs,
            log_tx,
            ActiveCollectionLog::error(message.clone()).for_task(task_log_id),
        );
        return Err(ReconTaskError::new(
            "active_tool_auto_install_failed",
            message,
        ));
    }

    if let Some(exec) = managed_tool_executable(tool, tools_dir) {
        if active_tool_binary_usable(&tool.id, &exec).await {
            emit_active_log(
                logs,
                log_tx,
                ActiveCollectionLog::info(format!(
                    "active_tool_auto_install_ready: tool={} executable={}",
                    tool.id, exec
                ))
                .for_task(task_log_id),
            );
            return Ok(exec);
        }
    }
    if let Some(exec) = golish_pentest::resolve_tool_executable(&tool.id, scan_tools, tools_dir) {
        if let Some(exec) = executable_candidate_path(&exec, tools_dir) {
            let exec = exec.display().to_string();
            if active_tool_binary_usable(&tool.id, &exec).await {
                emit_active_log(
                    logs,
                    log_tx,
                    ActiveCollectionLog::info(format!(
                        "active_tool_auto_install_ready: tool={} executable={}",
                        tool.id, exec
                    ))
                    .for_task(task_log_id),
                );
                return Ok(exec);
            }
        }
    }

    let message = format!(
        "active_tool_auto_install_failed: tool={} installed but executable was not found",
        tool.id
    );
    emit_active_log(
        logs,
        log_tx,
        ActiveCollectionLog::error(message.clone()).for_task(task_log_id),
    );
    Err(ReconTaskError::new(
        "active_tool_auto_install_failed",
        message,
    ))
}

fn executable_candidate_path(exec: &str, tools_dir: &Path) -> Option<PathBuf> {
    let path = Path::new(exec);
    if path.is_absolute() || exec.contains('/') || exec.contains('\\') {
        if path.is_file() {
            return Some(path.to_path_buf());
        }
        let tools_candidate = tools_dir.join(exec);
        if tools_candidate.is_file() {
            return Some(tools_candidate);
        }
        return None;
    }
    if command_on_path(exec) {
        return Some(path.to_path_buf());
    }
    None
}

async fn active_tool_wait_with_heartbeat(
    child: &mut tokio::process::Child,
    task: &ActiveTask,
    task_log_id: &str,
    timeout: Duration,
    log_tx: &Option<ActiveLogSender>,
) -> Result<std::process::ExitStatus, ActiveToolWaitError> {
    let started = Instant::now();
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;

    loop {
        tokio::select! {
            result = child.wait() => {
                return result.map_err(ActiveToolWaitError::Wait);
            }
            _ = heartbeat.tick() => {
                let elapsed = started.elapsed().as_secs();
                let log = ActiveCollectionLog::info(format!(
                    "active_tool_running: tool={} seed={} elapsed={}s timeout={}s",
                    task.tool_id,
                    task.seed,
                    elapsed,
                    timeout.as_secs()
                ))
                .for_task(task_log_id);
                emit_active_log_to_channel(log_tx, &log);
            }
            _ = &mut deadline => {
                let _ = child.kill().await;
                return Err(ActiveToolWaitError::Timeout);
            }
        }
    }
}

enum ActiveToolWaitError {
    Wait(std::io::Error),
    Timeout,
}

#[allow(clippy::too_many_arguments)]
async fn install_tool_via_tool_manager(
    tool: &ToolConfig,
    method: &str,
    source: &str,
    tools_dir: &Path,
    proxy_url: Option<&str>,
    github_token: Option<&str>,
    task_log_id: &str,
    logs: &mut Vec<ActiveCollectionLog>,
    log_tx: &Option<ActiveLogSender>,
) -> Result<(), String> {
    if method == "github" {
        match install_github_release_tool(
            tool,
            source,
            tools_dir,
            proxy_url,
            github_token,
            task_log_id,
            logs,
            log_tx,
        )
        .await
        {
            Ok(_) => return Ok(()),
            Err(error) => emit_active_log(
                logs,
                log_tx,
                ActiveCollectionLog::warn(format!(
                    "active_tool_auto_install_github_release_failed: tool={} error={} fallback={}",
                    tool.id,
                    error,
                    if tool.id == "urlfinder" {
                        "git_clone_go_build"
                    } else {
                        "git_clone"
                    }
                ))
                .for_task(task_log_id),
            ),
        }
    }

    let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<ActiveCollectionLog>::new()));
    let captured_for_log = captured.clone();
    let tx_for_log = log_tx.clone();
    let task_id_for_log = task_log_id.to_string();
    let log = move |message: &str| {
        let log = ActiveCollectionLog::info(format!("active_tool_auto_install_log: {message}"))
            .for_task(task_id_for_log.clone());
        emit_active_log_to_channel(&tx_for_log, &log);
        if let Ok(mut guard) = captured_for_log.lock() {
            guard.push(log);
        }
    };
    let result = golish_pentest::handlers::install_tool_from_config(
        method, source, &tool.name, tools_dir, &log, proxy_url,
    )
    .await;
    if let Ok(mut guard) = captured.lock() {
        logs.extend(guard.drain(..));
    }
    if result.success && tool.id == "urlfinder" {
        build_go_tool_if_possible(tool, tools_dir, logs, log_tx, task_log_id).await?;
        Ok(())
    } else if result.success {
        Ok(())
    } else {
        Err(result.message)
    }
}

async fn install_runtime_via_tool_manager(
    tool: &ToolConfig,
    runtime_type: &str,
    proxy_url: Option<&str>,
    task_log_id: &str,
    logs: &mut Vec<ActiveCollectionLog>,
    log_tx: &Option<ActiveLogSender>,
) -> Result<(), String> {
    let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<ActiveCollectionLog>::new()));
    let captured_for_log = captured.clone();
    let tx_for_log = log_tx.clone();
    let tool_id = tool.id.clone();
    let task_id_for_log = task_log_id.to_string();
    let log = move |message: &str| {
        let log = ActiveCollectionLog::info(format!("active_tool_auto_install_log: {message}"))
            .for_task(task_id_for_log.clone());
        tracing::info!(tool_id = %tool_id, message = %log.message, "organization_recon active runtime install");
        emit_active_log_to_channel(&tx_for_log, &log);
        if let Ok(mut guard) = captured_for_log.lock() {
            guard.push(log);
        }
    };
    let result =
        golish_pentest::handlers::install_runtime(runtime_type, &log, proxy_url, None).await;
    if let Ok(mut guard) = captured.lock() {
        logs.extend(guard.drain(..));
    }
    if result.success {
        Ok(())
    } else {
        Err(result.message)
    }
}

#[allow(clippy::too_many_arguments)]
async fn install_github_release_tool(
    tool: &ToolConfig,
    source: &str,
    tools_dir: &Path,
    proxy_url: Option<&str>,
    github_token: Option<&str>,
    task_log_id: &str,
    logs: &mut Vec<ActiveCollectionLog>,
    log_tx: &Option<ActiveLogSender>,
) -> Result<String, String> {
    let Some((owner, repo)) = source.split_once('/') else {
        return Err(format!("invalid GitHub source: {source}"));
    };
    emit_active_log(
        logs,
        log_tx,
        ActiveCollectionLog::info(format!(
            "active_tool_auto_install_github_release: tool={} repo={}/{}",
            tool.id, owner, repo
        ))
        .for_task(task_log_id),
    );
    let release =
        golish_pentest::github::fetch_latest_release(owner, repo, github_token, proxy_url)
            .await
            .map_err(|error| format!("fetch latest release failed: {error}"))?;
    let asset = select_github_asset(&release.assets)
        .ok_or_else(|| "no suitable GitHub release asset for this platform".to_string())?;
    emit_active_log(
        logs,
        log_tx,
        ActiveCollectionLog::info(format!(
            "active_tool_auto_install_download: tool={} asset={} version={}",
            tool.id, asset.name, release.tag_name
        ))
        .for_task(task_log_id),
    );

    std::fs::create_dir_all(tools_dir).map_err(|error| error.to_string())?;
    let tmp_dir = std::env::temp_dir()
        .join(ACTIVE_TOOL_INSTALL_TMP)
        .join(&tool.id);
    if tmp_dir.exists() {
        std::fs::remove_dir_all(&tmp_dir).map_err(|error| error.to_string())?;
    }
    std::fs::create_dir_all(&tmp_dir).map_err(|error| error.to_string())?;
    let tmp_file = tmp_dir.join(&asset.name);
    golish_pentest::github::download_file(
        &asset.browser_download_url,
        &tmp_file,
        proxy_url,
        None,
        None,
    )
    .await
    .map_err(|error| format!("download release asset failed: {error}"))?;

    let stable_dir = tools_dir.join(stable_tool_dir_name(tool));
    if stable_dir.exists() {
        std::fs::remove_dir_all(&stable_dir).map_err(|error| error.to_string())?;
    }
    std::fs::create_dir_all(&stable_dir).map_err(|error| error.to_string())?;
    if is_archive_name(&asset.name) {
        let extract_dir = tmp_dir.join("extract");
        std::fs::create_dir_all(&extract_dir).map_err(|error| error.to_string())?;
        golish_pentest::tool_package::extract_archive(&tmp_file, &extract_dir)
            .map_err(|error| format!("extract release asset failed: {error}"))?;
        let source_dir = unwrap_single_dir(&extract_dir);
        copy_dir_recursive(&source_dir, &stable_dir).map_err(|error| error.to_string())?;
    } else {
        let dest = stable_dir.join(&asset.name);
        std::fs::copy(&tmp_file, &dest).map_err(|error| error.to_string())?;
    }
    set_executable_permissions_recursive(&stable_dir);
    std::fs::remove_dir_all(&tmp_dir).ok();

    let Some(exec) = managed_tool_executable(tool, tools_dir) else {
        return Err(format!(
            "installed {} but no executable candidate was detected",
            tool.id
        ));
    };
    Ok(exec)
}

async fn build_go_tool_if_possible(
    tool: &ToolConfig,
    tools_dir: &Path,
    logs: &mut Vec<ActiveCollectionLog>,
    log_tx: &Option<ActiveLogSender>,
    task_log_id: &str,
) -> Result<(), String> {
    let source_dir = tools_dir.join(stable_tool_dir_name(tool));
    if !source_dir.join("go.mod").exists() || !source_dir.join("main.go").exists() {
        return Err(format!(
            "{} source clone does not contain go.mod/main.go; release binary is required",
            tool.id
        ));
    }
    let output = source_dir.join(executable_file_name(tool));
    emit_active_log(
        logs,
        log_tx,
        ActiveCollectionLog::info(format!(
            "active_tool_auto_install_log: building {} with go build -o {}",
            tool.id,
            output.display()
        ))
        .for_task(task_log_id),
    );
    let mut command = Command::new("go");
    command.arg("build").arg("-o").arg(&output).arg(".");
    command.current_dir(&source_dir);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let output_result = tokio::time::timeout(Duration::from_secs(600), command.output())
        .await
        .map_err(|_| format!("go build for {} timed out", tool.id))?
        .map_err(|error| format!("go build for {} failed to start: {error}", tool.id))?;
    emit_install_output_logs(logs, log_tx, task_log_id, "stdout", &output_result.stdout);
    emit_install_output_logs(logs, log_tx, task_log_id, "stderr", &output_result.stderr);
    if !output_result.status.success() {
        return Err(format!(
            "go build for {} exited with {:?}: {}",
            tool.id,
            output_result.status.code(),
            bytes_preview(&output_result.stderr)
        ));
    }
    set_executable_permission(&output);
    emit_active_log(
        logs,
        log_tx,
        ActiveCollectionLog::info(format!(
            "active_tool_auto_install_log: built {} executable at {}",
            tool.id,
            output.display()
        ))
        .for_task(task_log_id),
    );
    Ok(())
}

fn executable_file_name(tool: &ToolConfig) -> String {
    Path::new(&tool.executable)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| stable_tool_dir_name(tool))
}

fn emit_install_output_logs(
    logs: &mut Vec<ActiveCollectionLog>,
    log_tx: &Option<ActiveLogSender>,
    task_log_id: &str,
    stream: &str,
    bytes: &[u8],
) {
    let text = bytes_preview(bytes);
    if text == "(empty)" {
        return;
    }
    emit_active_log(
        logs,
        log_tx,
        ActiveCollectionLog::info(format!(
            "active_tool_auto_install_log: go build {stream}: {text}"
        ))
        .for_task(task_log_id),
    );
}

fn emit_active_log(
    logs: &mut Vec<ActiveCollectionLog>,
    log_tx: &Option<ActiveLogSender>,
    log: ActiveCollectionLog,
) {
    emit_active_log_to_channel(log_tx, &log);
    logs.push(log);
}

fn emit_active_log_to_channel(log_tx: &Option<ActiveLogSender>, log: &ActiveCollectionLog) {
    match log.level.as_str() {
        "error" => tracing::error!(message = %log.message, "organization_recon active log"),
        "warning" => tracing::warn!(message = %log.message, "organization_recon active log"),
        _ => tracing::info!(message = %log.message, "organization_recon active log"),
    }
    if let Some(tx) = log_tx {
        let _ = tx.send(log.clone());
    }
}

async fn read_active_stream<R>(
    mut reader: R,
    stream: &'static str,
    task_log_id: String,
    log_tx: Option<ActiveLogSender>,
) -> Vec<u8>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut pending = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(read) => {
                output.extend_from_slice(&buffer[..read]);
                pending.extend_from_slice(&buffer[..read]);
                drain_stream_lines(stream, &task_log_id, &log_tx, &mut pending);
                if pending.len() > 8192 {
                    emit_active_stream_line(stream, &task_log_id, &log_tx, &pending);
                    pending.clear();
                }
            }
            Err(error) => {
                let log = ActiveCollectionLog::warn(format!(
                    "active_tool_stream_read_failed: stream={stream} error={error}"
                ))
                .for_task(task_log_id.clone());
                emit_active_log_to_channel(&log_tx, &log);
                break;
            }
        }
    }
    if !pending.is_empty() {
        emit_active_stream_line(stream, &task_log_id, &log_tx, &pending);
    }
    output
}

fn drain_stream_lines(
    stream: &'static str,
    task_log_id: &str,
    log_tx: &Option<ActiveLogSender>,
    pending: &mut Vec<u8>,
) {
    while let Some(pos) = pending
        .iter()
        .position(|byte| *byte == b'\n' || *byte == b'\r')
    {
        let mut line = pending.drain(..=pos).collect::<Vec<_>>();
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        emit_active_stream_line(stream, task_log_id, log_tx, &line);
    }
}

fn emit_active_stream_line(
    stream: &'static str,
    task_log_id: &str,
    log_tx: &Option<ActiveLogSender>,
    line: &[u8],
) {
    let text = clean_stream_line(line);
    if text.is_empty() {
        return;
    }
    let message = format!("active_tool_{stream}: {text}");
    let log = if stream == "stderr" {
        ActiveCollectionLog::warn(message)
    } else {
        ActiveCollectionLog::info(message)
    }
    .for_task(task_log_id);
    emit_active_log_to_channel(log_tx, &log);
}

fn clean_stream_line(bytes: &[u8]) -> String {
    match decode_utf8_clean(bytes) {
        Ok(text) => {
            let text = text.trim();
            if text.chars().count() > 800 {
                format!("{}...", text.chars().take(800).collect::<String>())
            } else {
                text.to_string()
            }
        }
        Err(_) => format!("<non-utf8 {} byte(s)>", bytes.len()),
    }
}

async fn collect_active_stream(handle: Option<tokio::task::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    match handle {
        Some(handle) => handle.await.unwrap_or_default(),
        None => Vec::new(),
    }
}

fn is_amass_engine_timeout(task: &ActiveTask, stderr: &[u8]) -> bool {
    task.tool_id == "amass"
        && String::from_utf8_lossy(stderr)
            .to_ascii_lowercase()
            .contains("amass engine did not respond")
}

async fn active_tool_binary_usable(tool_id: &str, exec: &str) -> bool {
    match tool_id {
        "httpx" => {
            command_help_contains(exec, &["-h"], &["-json", "-td"]).await
                || command_help_contains(exec, &["-h"], &["projectdiscovery"]).await
        }
        "subfinder" => {
            command_help_contains(exec, &["-h"], &["subfinder"]).await
                || command_help_contains(exec, &["-h"], &["projectdiscovery"]).await
        }
        "urlfinder" => {
            command_help_contains(exec, &["-h"], &["urlfinder"]).await
                || command_help_contains(exec, &["-h"], &["-u", "-m"]).await
        }
        _ => true,
    }
}

async fn command_help_contains(exec: &str, args: &[&str], needles: &[&str]) -> bool {
    let mut command = Command::new(exec);
    command.args(args);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let Ok(Ok(output)) = tokio::time::timeout(Duration::from_secs(5), command.output()).await
    else {
        return false;
    };
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    needles
        .iter()
        .all(|needle| text.contains(&needle.to_ascii_lowercase()))
}

fn command_on_path(command: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let path = dir.join(command);
        if path.exists() {
            return true;
        }
        #[cfg(target_os = "windows")]
        {
            ["exe", "bat", "cmd", "ps1"]
                .iter()
                .any(|ext| dir.join(format!("{command}.{ext}")).exists())
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    })
}

fn managed_tool_executable(tool: &ToolConfig, tools_dir: &Path) -> Option<String> {
    let stable_dir = tools_dir.join(stable_tool_dir_name(tool));
    for candidate in explicit_executable_candidates(tool, tools_dir, &stable_dir) {
        if candidate.is_file() {
            return Some(candidate.display().to_string());
        }
    }
    if !stable_dir.exists() {
        return None;
    }
    let candidates = golish_pentest::tool_package::find_tool_executables(
        &stable_dir,
        Some(tool.runtime.as_str()),
    );
    let selected = select_executable_candidate(tool, &candidates)?;
    Some(stable_dir.join(selected).display().to_string())
}

fn explicit_executable_candidates(
    tool: &ToolConfig,
    tools_dir: &Path,
    stable_dir: &Path,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let executable = tool.executable.trim();
    if !executable.is_empty() {
        let path = Path::new(executable);
        if path.is_absolute() {
            candidates.push(path.to_path_buf());
        } else {
            candidates.push(tools_dir.join(path));
            if let Some(file_name) = path.file_name() {
                candidates.push(stable_dir.join(file_name));
            }
        }
    }
    candidates.push(stable_dir.join(&tool.id));
    candidates.push(stable_dir.join(stable_tool_dir_name(tool)));
    #[cfg(target_os = "windows")]
    {
        candidates.push(stable_dir.join(format!("{}.exe", tool.id)));
        candidates.push(stable_dir.join(format!("{}.exe", stable_tool_dir_name(tool))));
    }
    candidates
}

fn stable_tool_dir_name(tool: &ToolConfig) -> String {
    if tool.name.trim().is_empty() {
        tool.id.clone()
    } else {
        tool.name.trim().to_string()
    }
}

fn select_executable_candidate(tool: &ToolConfig, candidates: &[String]) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    let id = tool.id.to_ascii_lowercase();
    let name = tool.name.to_ascii_lowercase();
    candidates
        .iter()
        .find(|candidate| {
            let file = Path::new(candidate)
                .file_stem()
                .map(|name| name.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            file == id || file == name
        })
        .or_else(|| {
            candidates.iter().find(|candidate| {
                let lower = candidate.to_ascii_lowercase();
                lower.contains(&id) || (!name.is_empty() && lower.contains(&name))
            })
        })
        .or_else(|| candidates.first())
        .cloned()
}

fn select_github_asset(
    assets: &[golish_pentest::github::GitHubAsset],
) -> Option<golish_pentest::github::GitHubAsset> {
    let platform_terms: &[&str] = if cfg!(target_os = "macos") {
        &["darwin", "macos", "mac", "osx"]
    } else if cfg!(target_os = "windows") {
        &["windows", "win64", "win32", "win-"]
    } else {
        &["linux"]
    };
    let arch_terms: &[&str] = if cfg!(target_arch = "aarch64") {
        &["arm64", "aarch64"]
    } else {
        &["x86_64", "x64", "amd64", "64"]
    };

    let mut platform_assets: Vec<_> = assets
        .iter()
        .filter(|asset| !is_skippable_asset(&asset.name))
        .filter(|asset| {
            let lower = asset.name.to_ascii_lowercase();
            platform_terms.iter().any(|term| lower.contains(term))
                || (cfg!(target_os = "windows")
                    && (lower.ends_with(".exe") || lower.ends_with(".msi")))
        })
        .cloned()
        .collect();
    let arch_assets: Vec<_> = platform_assets
        .iter()
        .filter(|asset| {
            let lower = asset.name.to_ascii_lowercase();
            if cfg!(target_arch = "aarch64") {
                arch_terms.iter().any(|term| lower.contains(term))
            } else if lower.contains("arm64") || lower.contains("aarch64") {
                false
            } else {
                arch_terms.iter().any(|term| lower.contains(term))
            }
        })
        .cloned()
        .collect();
    if !arch_assets.is_empty() {
        platform_assets = arch_assets;
    }

    platform_assets
        .iter()
        .find(|asset| is_archive_name(&asset.name))
        .or_else(|| platform_assets.first())
        .or_else(|| {
            assets
                .iter()
                .find(|asset| !is_skippable_asset(&asset.name) && is_archive_name(&asset.name))
        })
        .or_else(|| assets.iter().find(|asset| !is_skippable_asset(&asset.name)))
        .cloned()
}

fn is_skippable_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".txt")
        || lower.ends_with(".md")
        || lower.ends_with(".sha256")
        || lower.ends_with(".sha512")
        || lower.ends_with(".asc")
        || lower.ends_with(".sig")
        || lower.contains("checksum")
        || lower.contains("sbom")
}

fn is_archive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".tar.gz") || lower.ends_with(".tgz")
}

fn unwrap_single_dir(dir: &Path) -> PathBuf {
    let Ok(mut entries) = std::fs::read_dir(dir) else {
        return dir.to_path_buf();
    };
    let first = entries.next();
    if entries.next().is_some() {
        return dir.to_path_buf();
    }
    if let Some(Ok(entry)) = first {
        if entry.file_type().map(|ty| ty.is_dir()).unwrap_or(false) {
            return entry.path();
        }
    }
    dir.to_path_buf()
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn set_executable_permissions_recursive(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            set_executable_permissions_recursive(&path);
        } else if should_mark_executable(&path) {
            set_executable_permission(&path);
        }
    }
}

fn should_mark_executable(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    ext.is_empty() || matches!(ext.as_str(), "exe" | "sh" | "command")
}

#[cfg(unix)]
fn set_executable_permission(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        let _ = std::fs::set_permissions(path, permissions);
    }
}

#[cfg(not(unix))]
fn set_executable_permission(_path: &Path) {}

#[allow(clippy::too_many_arguments)]
async fn run_active_task(
    scan_tools: &[ToolConfig],
    tools_dir: &Path,
    proxy_url: Option<&str>,
    github_token: Option<&str>,
    run_id: &str,
    active_dir: &Path,
    scope: &ActiveScopeSet,
    task: ActiveTask,
    log_tx: Option<ActiveLogSender>,
) -> Result<ActiveTaskResult, GolishError> {
    let task_log_id = safe_task_name(&task.tool_id, &task.seed);
    let task_dir = active_dir.join(&task_log_id);
    let mut logs = Vec::new();
    let Some(tool) = scan_tools.iter().find(|tool| tool.id == task.tool_id) else {
        let message = format!(
            "active_tool_config_missing: tool={} seed={} config not found",
            task.tool_id, task.seed
        );
        tracing::warn!(
            tool_id = task.tool_id,
            seed = %task.seed,
            "organization_recon active tool config missing"
        );
        emit_active_log(
            &mut logs,
            &log_tx,
            ActiveCollectionLog::error(message).for_task(&task_log_id),
        );
        let errors = vec![ReconTaskError::new(
            "active_tool_config_missing",
            format!("tool config '{}' not found", task.tool_id),
        )];
        let mut artifacts = Vec::new();
        write_active_task_manifest(
            ActiveTaskManifestInput {
                task_dir: &task_dir,
                run_id,
                task: &task,
                status: ReconTaskStatus::Failed,
                exit_code: None,
                record_count: 0,
                errors: &errors,
            },
            &mut artifacts,
        )?;
        return Ok(ActiveTaskResult {
            records: Vec::new(),
            errors,
            artifacts,
        });
    };
    let exec = match resolve_or_install_active_tool(
        tool,
        scan_tools,
        tools_dir,
        proxy_url,
        github_token,
        &task_log_id,
        &mut logs,
        &log_tx,
    )
    .await
    {
        Ok(exec) => exec,
        Err(error) => {
            let errors = vec![error];
            let mut artifacts = Vec::new();
            write_active_task_manifest(
                ActiveTaskManifestInput {
                    task_dir: &task_dir,
                    run_id,
                    task: &task,
                    status: ReconTaskStatus::Failed,
                    exit_code: None,
                    record_count: 0,
                    errors: &errors,
                },
                &mut artifacts,
            )?;
            return Ok(ActiveTaskResult {
                records: Vec::new(),
                errors,
                artifacts,
            });
        }
    };

    let argv = serde_json::to_vec_pretty(&serde_json::json!({
        "toolId": &task.tool_id,
        "executable": &exec,
        "args": &task.args,
        "timeoutSecs": task.timeout_secs,
    }))
    .map_err(|error| GolishError::Internal(format!("serialize active argv failed: {error}")))?;
    let mut artifacts = vec![write_raw_bytes(&task_dir, "raw/argv.json", &argv, "argv")?];

    let mut command = Command::new(&exec);
    command.args(&task.args);
    command.current_dir(&task_dir);
    command.kill_on_drop(true);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let spawn_message = format!(
        "active_tool_spawn: tool={} seed={} executable={} args={}",
        task.tool_id,
        task.seed,
        exec,
        shell_words(&task.args)
    );
    tracing::info!(
        tool_id = %task.tool_id,
        seed = %task.seed,
        executable = %exec,
        args = ?task.args,
        "organization_recon active tool spawn"
    );
    emit_active_log(
        &mut logs,
        &log_tx,
        ActiveCollectionLog::info(spawn_message).for_task(&task_log_id),
    );

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = format!(
                "active_tool_spawn_failed: tool={} seed={} executable={} error={error}",
                task.tool_id, task.seed, exec
            );
            tracing::warn!(
                tool_id = %task.tool_id,
                seed = %task.seed,
                executable = %exec,
                error = %error,
                "organization_recon active tool spawn failed"
            );
            emit_active_log(
                &mut logs,
                &log_tx,
                ActiveCollectionLog::error(message).for_task(&task_log_id),
            );
            let errors = vec![ReconTaskError::new(
                "active_tool_spawn_failed",
                format!("{} spawn failed: {error}", task.tool_id),
            )];
            write_active_task_manifest(
                ActiveTaskManifestInput {
                    task_dir: &task_dir,
                    run_id,
                    task: &task,
                    status: ReconTaskStatus::Failed,
                    exit_code: None,
                    record_count: 0,
                    errors: &errors,
                },
                &mut artifacts,
            )?;
            return Ok(ActiveTaskResult {
                records: Vec::new(),
                errors,
                artifacts,
            });
        }
    };

    let stdout_reader = child.stdout.take().map(|stdout| {
        tokio::spawn(read_active_stream(
            stdout,
            "stdout",
            task_log_id.clone(),
            log_tx.clone(),
        ))
    });
    let stderr_reader = child.stderr.take().map(|stderr| {
        tokio::spawn(read_active_stream(
            stderr,
            "stderr",
            task_log_id.clone(),
            log_tx.clone(),
        ))
    });
    let timeout = Duration::from_secs(task.timeout_secs.clamp(1, 1800));
    let process_status =
        match active_tool_wait_with_heartbeat(&mut child, &task, &task_log_id, timeout, &log_tx)
            .await
        {
            Ok(status) => status,
            Err(ActiveToolWaitError::Wait(error)) => {
                let message = format!(
                    "active_tool_wait_failed: tool={} seed={} error={error}",
                    task.tool_id, task.seed
                );
                tracing::warn!(
                    tool_id = %task.tool_id,
                    seed = %task.seed,
                    error = %error,
                    "organization_recon active tool wait failed"
                );
                emit_active_log(
                    &mut logs,
                    &log_tx,
                    ActiveCollectionLog::error(message).for_task(&task_log_id),
                );
                let errors = vec![ReconTaskError::new(
                    "active_tool_wait_failed",
                    format!("{} wait failed: {error}", task.tool_id),
                )];
                write_active_task_manifest(
                    ActiveTaskManifestInput {
                        task_dir: &task_dir,
                        run_id,
                        task: &task,
                        status: ReconTaskStatus::Failed,
                        exit_code: None,
                        record_count: 0,
                        errors: &errors,
                    },
                    &mut artifacts,
                )?;
                return Ok(ActiveTaskResult {
                    records: Vec::new(),
                    errors,
                    artifacts,
                });
            }
            Err(ActiveToolWaitError::Timeout) => {
                let message = format!(
                    "active_tool_timeout: tool={} seed={} timeout={}s",
                    task.tool_id, task.seed, task.timeout_secs
                );
                tracing::warn!(
                    tool_id = %task.tool_id,
                    seed = %task.seed,
                    timeout_secs = task.timeout_secs,
                    "organization_recon active tool timeout"
                );
                emit_active_log(
                    &mut logs,
                    &log_tx,
                    ActiveCollectionLog::error(message).for_task(&task_log_id),
                );
                let errors = vec![ReconTaskError::new(
                    "active_tool_timeout",
                    format!("{} timed out after {}s", task.tool_id, task.timeout_secs),
                )];
                write_active_task_manifest(
                    ActiveTaskManifestInput {
                        task_dir: &task_dir,
                        run_id,
                        task: &task,
                        status: ReconTaskStatus::Failed,
                        exit_code: None,
                        record_count: 0,
                        errors: &errors,
                    },
                    &mut artifacts,
                )?;
                return Ok(ActiveTaskResult {
                    records: Vec::new(),
                    errors,
                    artifacts,
                });
            }
        };
    let stdout = collect_active_stream(stdout_reader).await;
    let stderr = collect_active_stream(stderr_reader).await;

    let stdout_artifact = write_raw_bytes(&task_dir, "raw/stdout.log", &stdout, "stdout")?;
    let stderr_artifact = write_raw_bytes(&task_dir, "raw/stderr.log", &stderr, "stderr")?;
    let stdout_path = stdout_artifact.path.clone();
    artifacts.push(stdout_artifact);
    artifacts.push(stderr_artifact);

    let mut errors = Vec::new();
    if !process_status.success() {
        let exit_code = process_status.code().unwrap_or(-1);
        let stderr_preview = bytes_preview(&stderr);
        if is_amass_engine_timeout(&task, &stderr) {
            let message = format!(
                "active_tool_checked_empty: tool={} seed={} reason=amass_engine_timeout stderr={}",
                task.tool_id, task.seed, stderr_preview
            );
            tracing::warn!(
                tool_id = %task.tool_id,
                seed = %task.seed,
                exit_code,
                stderr = %stderr_preview,
                "organization_recon active tool returned known checked-empty condition"
            );
            emit_active_log(
                &mut logs,
                &log_tx,
                ActiveCollectionLog::warn(message).for_task(&task_log_id),
            );
        } else {
            let message = format!(
                "active_tool_nonzero_exit: tool={} seed={} exit_code={} stderr={}",
                task.tool_id, task.seed, exit_code, stderr_preview
            );
            tracing::warn!(
                tool_id = %task.tool_id,
                seed = %task.seed,
                exit_code,
                stderr = %stderr_preview,
                "organization_recon active tool nonzero exit"
            );
            emit_active_log(
                &mut logs,
                &log_tx,
                ActiveCollectionLog::error(message).for_task(&task_log_id),
            );
            errors.push(ReconTaskError::new(
                "active_tool_nonzero_exit",
                format!(
                    "{} exited with {:?}",
                    task.tool_id,
                    process_status.code().unwrap_or(-1)
                ),
            ));
        }
    }

    let mut records = match decode_utf8_clean(&stdout) {
        Ok(stdout) => parse_records(run_id, &task, scope, &stdout, &stdout_path),
        Err(error) => {
            let message = format!(
                "active_tool_output_decode_failed: tool={} seed={} {}",
                task.tool_id, task.seed, error.message
            );
            tracing::warn!(
                tool_id = task.tool_id,
                seed = %task.seed,
                error = %error.message,
                "organization_recon active tool output decode failed"
            );
            emit_active_log(
                &mut logs,
                &log_tx,
                ActiveCollectionLog::error(message).for_task(&task_log_id),
            );
            errors.push(error);
            Vec::new()
        }
    };
    if task.tool_id == "urlfinder" {
        match collect_urlfinder_records(run_id, &task, scope, &task_dir, &mut artifacts) {
            Ok(mut parsed) => records.append(&mut parsed),
            Err(error) => {
                let message = format!(
                    "active_tool_output_parse_failed: tool={} seed={} {}",
                    task.tool_id, task.seed, error.message
                );
                tracing::warn!(
                    tool_id = %task.tool_id,
                    seed = %task.seed,
                    error = %error.message,
                    "organization_recon active tool output parse failed"
                );
                emit_active_log(
                    &mut logs,
                    &log_tx,
                    ActiveCollectionLog::error(message).for_task(&task_log_id),
                );
                errors.push(error);
            }
        }
    }
    let status = if !errors.is_empty() {
        ReconTaskStatus::Failed
    } else if records.is_empty() {
        ReconTaskStatus::CheckedEmpty
    } else {
        ReconTaskStatus::Completed
    };
    let finished_message = format!(
        "active_tool_finished: tool={} seed={} status={:?} exit_code={:?} records={}",
        task.tool_id,
        task.seed,
        status,
        process_status.code(),
        records.len()
    );
    if matches!(status, ReconTaskStatus::Failed) {
        tracing::warn!(
            tool_id = %task.tool_id,
            seed = %task.seed,
            status = ?status,
            record_count = records.len(),
            "organization_recon active tool finished with errors"
        );
        emit_active_log(
            &mut logs,
            &log_tx,
            ActiveCollectionLog::warn(finished_message).for_task(&task_log_id),
        );
    } else {
        tracing::info!(
            tool_id = %task.tool_id,
            seed = %task.seed,
            status = ?status,
            record_count = records.len(),
            "organization_recon active tool finished"
        );
        emit_active_log(
            &mut logs,
            &log_tx,
            ActiveCollectionLog::info(finished_message).for_task(&task_log_id),
        );
    }
    write_active_task_manifest(
        ActiveTaskManifestInput {
            task_dir: &task_dir,
            run_id,
            task: &task,
            status,
            exit_code: process_status.code(),
            record_count: records.len(),
            errors: &errors,
        },
        &mut artifacts,
    )?;

    Ok(ActiveTaskResult {
        records,
        errors,
        artifacts,
    })
}

fn write_active_task_manifest(
    input: ActiveTaskManifestInput<'_>,
    artifacts: &mut Vec<ReconArtifactRef>,
) -> Result<(), GolishError> {
    let mut manifest = ReconTaskManifest::new(
        input.run_id,
        safe_task_name(&input.task.tool_id, &input.task.seed),
        "active_collection",
        &input.task.tool_id,
    );
    manifest.status = input.status;
    manifest.exit_code = input.exit_code;
    manifest.artifacts = artifacts.clone();
    manifest.record_count = input.record_count;
    manifest.checked_empty = matches!(manifest.status, ReconTaskStatus::CheckedEmpty);
    manifest.errors = input.errors.to_vec();

    let manifest_path = write_json_manifest(input.task_dir, &manifest)?;
    artifacts.push(ReconArtifactRef {
        bytes: std::fs::metadata(&manifest_path)?.len(),
        kind: "task_manifest".into(),
        path: manifest_path.display().to_string(),
    });
    Ok(())
}

fn planned_tasks(scope: &ActiveScopeSet) -> Vec<ActiveTask> {
    let mut tasks = Vec::new();
    for root in scope.roots.iter().take(MAX_ACTIVE_SEEDS) {
        tasks.push(ActiveTask {
            tool_id: "subfinder".into(),
            seed: root.clone(),
            args: vec!["-d".into(), root.clone(), "-silent".into()],
            timeout_secs: 900,
        });
        tasks.push(ActiveTask {
            tool_id: "amass".into(),
            seed: root.clone(),
            args: vec![
                "enum".into(),
                "-d".into(),
                root.clone(),
                "-passive".into(),
                "-silent".into(),
            ],
            timeout_secs: 1800,
        });
    }

    for host in scope.hosts.iter().take(MAX_ACTIVE_SEEDS) {
        tasks.push(ActiveTask {
            tool_id: "nmap".into(),
            seed: host.clone(),
            args: vec![
                host.clone(),
                "-p".into(),
                NMAP_COMMON_PORTS.into(),
                "--open".into(),
                "-n".into(),
                "--max-retries".into(),
                "1".into(),
                "--host-timeout".into(),
                format!("{NMAP_TIMEOUT_SECS}s"),
                "-T3".into(),
            ],
            timeout_secs: NMAP_TIMEOUT_SECS + 10,
        });
        tasks.push(ActiveTask {
            tool_id: "httpx".into(),
            seed: host.clone(),
            args: vec![
                "-u".into(),
                host.clone(),
                "-json".into(),
                "-silent".into(),
                "-td".into(),
                "-title".into(),
                "-server".into(),
            ],
            timeout_secs: 600,
        });
    }
    for url in scope.urls.iter().take(MAX_ACTIVE_SEEDS) {
        let mut args = vec![
            "-u".into(),
            url.clone(),
            "-s".into(),
            "all".into(),
            "-m".into(),
            "3".into(),
            "-o".into(),
            ".".into(),
        ];
        if let Some(host) = host_from_target_value(url) {
            if let Some(root) = scope.root_for_host(&host) {
                args.push("-d".into());
                args.push(root.replace('.', "\\."));
            }
        }
        tasks.push(ActiveTask {
            tool_id: "urlfinder".into(),
            seed: url.clone(),
            args,
            timeout_secs: 900,
        });
    }
    tasks
}

fn parse_records(
    run_id: &str,
    task: &ActiveTask,
    scope: &ActiveScopeSet,
    stdout: &str,
    raw_artifact_path: &str,
) -> Vec<NormalizedReconRecord> {
    match task.tool_id.as_str() {
        "subfinder" | "amass" => stdout
            .lines()
            .filter_map(|line| {
                let host = line.trim().trim_end_matches('.');
                if scope.accepts_host(host) {
                    normalized_active_record(
                        run_id,
                        task,
                        ReconRecordKind::Domain,
                        host,
                        json_attrs("host", host),
                        raw_artifact_path,
                    )
                } else {
                    None
                }
            })
            .collect(),
        "nmap" => parse_nmap_records(run_id, task, scope, stdout, raw_artifact_path),
        "httpx" => parse_httpx_records(run_id, task, scope, stdout, raw_artifact_path),
        _ => Vec::new(),
    }
}

fn parse_nmap_records(
    run_id: &str,
    task: &ActiveTask,
    scope: &ActiveScopeSet,
    stdout: &str,
    raw_artifact_path: &str,
) -> Vec<NormalizedReconRecord> {
    let mut current_host = task.seed.as_str();
    let mut records = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Nmap scan report for ") {
            current_host = rest
                .split_whitespace()
                .next()
                .unwrap_or(current_host)
                .trim_matches(['(', ')']);
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(port_proto) = parts.next() else {
            continue;
        };
        let Some(state) = parts.next() else {
            continue;
        };
        let service = parts.next().unwrap_or_default();
        if state != "open" || !scope.accepts_host(current_host) {
            continue;
        }
        let value = format!("{current_host}:{port_proto}");
        records.push_opt(normalized_active_record(
            run_id,
            task,
            ReconRecordKind::Port,
            &value,
            serde_json::json!({
                "host": current_host,
                "portProtocol": port_proto,
                "service": service,
            }),
            raw_artifact_path,
        ));
    }
    records
}

fn parse_httpx_records(
    run_id: &str,
    task: &ActiveTask,
    scope: &ActiveScopeSet,
    stdout: &str,
    raw_artifact_path: &str,
) -> Vec<NormalizedReconRecord> {
    stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| {
            let url = value
                .get("url")
                .or_else(|| value.get("input"))
                .and_then(Value::as_str)?;
            let host = url::Url::parse(url).ok()?.host_str()?.to_string();
            if !scope.accepts_host(&host) {
                return None;
            }
            normalized_active_record(
                run_id,
                task,
                ReconRecordKind::Url,
                url,
                serde_json::json!({
                    "statusCode": value.get("status_code").or_else(|| value.get("status-code")),
                    "title": value.get("title"),
                    "webserver": value.get("webserver"),
                    "technologies": value.get("tech"),
                }),
                raw_artifact_path,
            )
        })
        .collect()
}

fn collect_urlfinder_records(
    run_id: &str,
    task: &ActiveTask,
    scope: &ActiveScopeSet,
    task_dir: &Path,
    artifacts: &mut Vec<ReconArtifactRef>,
) -> Result<Vec<NormalizedReconRecord>, ReconTaskError> {
    let mut paths = Vec::new();
    collect_json_paths(task_dir, &mut paths).map_err(|error| {
        ReconTaskError::new(
            "active_tool_output_parse_failed",
            format!("scan URLFinder output failed: {error}"),
        )
    })?;

    let mut records = Vec::new();
    for path in paths {
        if should_skip_urlfinder_json(task_dir, &path) {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|error| {
            ReconTaskError::new(
                "active_tool_output_parse_failed",
                format!("read URLFinder output {} failed: {error}", path.display()),
            )
        })?;
        let value = serde_json::from_slice::<Value>(&bytes).map_err(|error| {
            ReconTaskError::new(
                "active_tool_output_parse_failed",
                format!("parse URLFinder output {} failed: {error}", path.display()),
            )
        })?;
        let raw_path = path.display().to_string();
        artifacts.push(ReconArtifactRef {
            bytes: bytes.len() as u64,
            kind: "urlfinder_json".into(),
            path: raw_path.clone(),
        });
        records.extend(parse_urlfinder_value(
            run_id, task, scope, &value, &raw_path,
        ));
    }
    Ok(records)
}

fn collect_json_paths(dir: &Path, paths: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_paths(&path, paths)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    Ok(())
}

fn should_skip_urlfinder_json(task_dir: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(task_dir) else {
        return false;
    };
    rel.components()
        .next()
        .map(|component| component.as_os_str() == "raw")
        .unwrap_or(false)
        || rel.file_name().and_then(|name| name.to_str()) == Some("manifest.json")
}

fn parse_urlfinder_value(
    run_id: &str,
    task: &ActiveTask,
    scope: &ActiveScopeSet,
    value: &Value,
    raw_artifact_path: &str,
) -> Vec<NormalizedReconRecord> {
    let mut records = Vec::new();
    for (field, source_kind) in [
        ("url", "url"),
        ("urlOther", "url_other"),
        ("js", "js"),
        ("jsOther", "js_other"),
        ("fuzz", "fuzz"),
    ] {
        if let Some(items) = value.get(field).and_then(Value::as_array) {
            for item in items {
                let Some(url) = link_url(item) else {
                    continue;
                };
                if !scope.accepts_url(url) {
                    continue;
                }
                records.push_opt(normalized_active_record(
                    run_id,
                    task,
                    ReconRecordKind::Url,
                    url,
                    serde_json::json!({
                        "source": "urlfinder",
                        "kind": source_kind,
                        "status": item.get("Status").or_else(|| item.get("status")),
                        "size": item.get("Size").or_else(|| item.get("size")),
                        "title": item.get("Title").or_else(|| item.get("title")),
                        "redirect": item.get("Redirect").or_else(|| item.get("redirect")),
                        "finderSource": item.get("Source").or_else(|| item.get("source")),
                    }),
                    raw_artifact_path,
                ));
            }
        }
    }

    if let Some(domains) = value.get("domain").and_then(Value::as_array) {
        for domain in domains.iter().filter_map(Value::as_str) {
            let domain = domain.trim().trim_end_matches('.');
            if !scope.accepts_host(domain) {
                continue;
            }
            records.push_opt(normalized_active_record(
                run_id,
                task,
                ReconRecordKind::Domain,
                domain,
                serde_json::json!({
                    "source": "urlfinder",
                    "kind": "domain",
                }),
                raw_artifact_path,
            ));
        }
    }

    if let Some(info) = value.get("info").and_then(Value::as_object) {
        for (category, entries) in info {
            let Some(entries) = entries.as_array() else {
                continue;
            };
            for entry in entries {
                let Some(leak_value) = info_entry_value(category, entry) else {
                    continue;
                };
                let leak_value = leak_value.trim();
                if leak_value.is_empty() {
                    continue;
                }
                records.push_opt(normalized_active_record(
                    run_id,
                    task,
                    ReconRecordKind::Leak,
                    leak_value,
                    serde_json::json!({
                        "source": "urlfinder",
                        "category": category,
                        "finderSource": entry.get("Source").or_else(|| entry.get("source")),
                    }),
                    raw_artifact_path,
                ));
            }
        }
    }
    records
}

fn link_url(value: &Value) -> Option<&str> {
    value
        .get("Url")
        .or_else(|| value.get("url"))
        .and_then(Value::as_str)
}

fn info_entry_value<'a>(category: &str, value: &'a Value) -> Option<&'a str> {
    let lower_category = category.to_ascii_lowercase();
    value
        .get(category)
        .or_else(|| value.get(lower_category.as_str()))
        .or_else(|| {
            value.as_object().and_then(|object| {
                object
                    .iter()
                    .find(|(key, item)| key.as_str() != "Source" && item.as_str().is_some())
                    .map(|(_, item)| item)
            })
        })
        .and_then(Value::as_str)
}

fn normalized_active_record(
    run_id: &str,
    task: &ActiveTask,
    kind: ReconRecordKind,
    value: &str,
    attributes: Value,
    raw_artifact_path: &str,
) -> Option<NormalizedReconRecord> {
    let key = normalize_record_key(&kind, value).ok()?;
    Some(NormalizedReconRecord {
        record_id: key.clone(),
        kind,
        key,
        value: value.into(),
        attributes,
        evidence: vec![ReconEvidenceRef {
            source_id: format!("active/{}", task.tool_id),
            run_id: run_id.into(),
            task_id: safe_task_name(&task.tool_id, &task.seed),
            raw_artifact_path: raw_artifact_path.into(),
        }],
    })
}

fn json_attrs(key: &str, value: &str) -> Value {
    serde_json::json!({ key: value })
}

fn shell_words(args: &[String]) -> String {
    if args.is_empty() {
        return "(none)".into();
    }
    args.join(" ")
}

fn bytes_preview(bytes: &[u8]) -> String {
    let preview = String::from_utf8_lossy(bytes);
    let preview = preview.trim();
    if preview.is_empty() {
        return "(empty)".into();
    }
    preview.chars().take(240).collect()
}

impl ActiveScopeSet {
    fn from_targets(targets: &[Target]) -> Self {
        let mut scope = Self::default();
        for value in targets.iter().map(|target| target.value.trim()) {
            if value.is_empty() {
                continue;
            }
            if let Some(url) = normalized_url_seed(value) {
                scope.urls.insert(url);
            }
            if let Some(host) = host_from_target_value(value) {
                scope.hosts.insert(host.clone());
                if looks_like_domain(&host) {
                    scope.roots.insert(host.clone());
                }
                if normalized_url_seed(value).is_none() {
                    scope.urls.insert(format!("https://{host}"));
                }
            }
        }
        scope
    }

    fn accepts_host(&self, host: &str) -> bool {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if self.hosts.contains(&host) {
            return true;
        }
        self.roots
            .iter()
            .any(|root| host == *root || host.ends_with(&format!(".{root}")))
    }

    fn accepts_url(&self, value: &str) -> bool {
        host_from_target_value(value)
            .map(|host| self.accepts_host(&host))
            .unwrap_or(false)
    }

    fn root_for_host(&self, host: &str) -> Option<&str> {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        self.roots
            .iter()
            .filter(|root| {
                let root = root.as_str();
                host == root || host.ends_with(&format!(".{root}"))
            })
            .max_by_key(|root| root.len())
            .map(String::as_str)
    }
}

fn normalized_url_seed(value: &str) -> Option<String> {
    let mut url = url::Url::parse(value).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let _ = url.set_host(Some(&host));
    url.set_fragment(None);
    Some(url.to_string())
}

fn host_from_target_value(value: &str) -> Option<String> {
    if let Ok(url) = url::Url::parse(value) {
        return url.host_str().map(|host| host.to_ascii_lowercase());
    }
    if looks_like_domain(value) || value.parse::<std::net::IpAddr>().is_ok() {
        return Some(value.trim().trim_end_matches('.').to_ascii_lowercase());
    }
    None
}

fn looks_like_domain(value: &str) -> bool {
    let value = value.trim().trim_end_matches('.');
    if value.contains(char::is_whitespace) || !value.contains('.') {
        return false;
    }
    value.split('.').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    })
}

fn safe_task_name(tool_id: &str, seed: &str) -> String {
    let seed: String = seed
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    format!("{tool_id}-{seed}")
}

trait PushOpt<T> {
    fn push_opt(&mut self, value: Option<T>);
}

impl<T> PushOpt<T> for Vec<T> {
    fn push_opt(&mut self, value: Option<T>) {
        if let Some(value) = value {
            self.push(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(value: &str) -> Target {
        Target {
            id: uuid::Uuid::new_v4().to_string(),
            name: value.into(),
            target_type: crate::targets::TargetType::Domain,
            value: value.into(),
            tags: Vec::new(),
            notes: String::new(),
            scope: crate::targets::Scope::InScope,
            status: crate::targets::TargetStatus::New,
            grp: "default".into(),
            owner: String::new(),
            time_window_start: None,
            time_window_end: None,
            organization_id: None,
            source: "fixture".into(),
            parent_id: None,
            ports: Vec::new(),
            real_ip: String::new(),
            cdn_waf: String::new(),
            http_title: String::new(),
            http_status: None,
            webserver: String::new(),
            os_info: String::new(),
            content_type: String::new(),
            liveness_state: None,
            liveness_reason: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn scope_accepts_only_exact_or_subdomain() {
        let scope = ActiveScopeSet::from_targets(&[target("example.com")]);

        assert!(scope.accepts_host("example.com"));
        assert!(scope.accepts_host("www.example.com"));
        assert!(!scope.accepts_host("badexample.com"));
    }

    #[test]
    fn subfinder_parser_filters_out_of_scope_hosts() {
        let scope = ActiveScopeSet::from_targets(&[target("example.com")]);
        let task = ActiveTask {
            tool_id: "subfinder".into(),
            seed: "example.com".into(),
            args: Vec::new(),
            timeout_secs: 1,
        };

        let records = parse_records(
            "run",
            &task,
            &scope,
            "www.example.com\nbadexample.com\n",
            "raw/stdout.log",
        );

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].value, "www.example.com");
    }

    #[test]
    fn httpx_parser_emits_url_record() {
        let scope = ActiveScopeSet::from_targets(&[target("example.com")]);
        let task = ActiveTask {
            tool_id: "httpx".into(),
            seed: "example.com".into(),
            args: Vec::new(),
            timeout_secs: 1,
        };

        let records = parse_records(
            "run",
            &task,
            &scope,
            r#"{"url":"https://www.example.com","status_code":200,"title":"OK"}"#,
            "raw/stdout.log",
        );

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, ReconRecordKind::Url);
    }

    #[test]
    fn urlfinder_parser_filters_external_urls_and_domains() {
        let scope = ActiveScopeSet::from_targets(&[target("example.com")]);
        let task = ActiveTask {
            tool_id: "urlfinder".into(),
            seed: "https://www.example.com".into(),
            args: Vec::new(),
            timeout_secs: 1,
        };
        let value = serde_json::json!({
            "url": [
                {"Url": "https://www.example.com/a", "Status": "200", "Title": "OK"},
                {"Url": "https://github.com/example/leak", "Status": "200"}
            ],
            "js": [
                {"Url": "https://static.example.com/app.js", "Status": "200"}
            ],
            "domain": ["www.example.com", "github.com"],
            "info": {
                "Email": [{"Email": "security@example.com", "Source": "https://www.example.com/a"}]
            }
        });

        let records = parse_urlfinder_value("run", &task, &scope, &value, "urlfinder.json");
        let values = records
            .iter()
            .map(|record| record.value.as_str())
            .collect::<Vec<_>>();

        assert!(values.contains(&"https://www.example.com/a"));
        assert!(values.contains(&"https://static.example.com/app.js"));
        assert!(values.contains(&"www.example.com"));
        assert!(values.contains(&"security@example.com"));
        assert!(!values.iter().any(|value| value.contains("github.com")));
    }

    #[test]
    fn planned_tasks_include_urlfinder_for_domain_targets() {
        let scope = ActiveScopeSet::from_targets(&[target("example.com")]);
        let tasks = planned_tasks(&scope);

        assert!(tasks
            .iter()
            .any(|task| task.tool_id == "urlfinder" && task.seed == "https://example.com"));
    }

    #[test]
    fn planned_nmap_task_uses_common_ports_and_short_timeout() {
        let scope = ActiveScopeSet::from_targets(&[target("example.com")]);
        let tasks = planned_tasks(&scope);
        let nmap = tasks
            .iter()
            .find(|task| task.tool_id == "nmap" && task.seed == "example.com")
            .unwrap();

        assert_eq!(nmap.timeout_secs, NMAP_TIMEOUT_SECS + 10);
        assert!(nmap
            .args
            .windows(2)
            .any(|pair| pair[0] == "-p" && pair[1] == NMAP_COMMON_PORTS));
        assert!(nmap.args.iter().any(|arg| arg == "--open"));
        assert!(nmap.args.iter().any(|arg| arg == "--host-timeout"));
        assert!(!nmap.args.iter().any(|arg| arg == "--top-ports"));
    }

    #[test]
    fn urlfinder_explicit_candidates_include_built_binary_path() {
        let tool = ToolConfig {
            id: "urlfinder".into(),
            name: "URLFinder".into(),
            executable: "URLFinder/URLFinder".into(),
            runtime: "native".into(),
            ..Default::default()
        };
        let tools_dir = Path::new("/tmp/golish-tools");
        let stable_dir = tools_dir.join("URLFinder");

        let candidates = explicit_executable_candidates(&tool, tools_dir, &stable_dir);

        assert!(candidates
            .iter()
            .any(|path| path == &tools_dir.join("URLFinder/URLFinder")));
        assert!(candidates
            .iter()
            .any(|path| path == &stable_dir.join("URLFinder")));
    }

    #[test]
    fn executable_candidate_prefers_urlfinder_binary_over_source_files() {
        let tool = ToolConfig {
            id: "urlfinder".into(),
            name: "URLFinder".into(),
            executable: "URLFinder/URLFinder".into(),
            runtime: "native".into(),
            ..Default::default()
        };

        let selected = select_executable_candidate(
            &tool,
            &["main.go".into(), "README.md".into(), "URLFinder".into()],
        );

        assert_eq!(selected.as_deref(), Some("URLFinder"));
    }

    #[test]
    fn executable_candidate_path_rejects_directory_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let tools_dir = dir.path();
        let tool_dir = tools_dir.join("subfinder");
        std::fs::create_dir_all(&tool_dir).unwrap();

        assert!(executable_candidate_path(&tool_dir.display().to_string(), tools_dir).is_none());

        let executable = tool_dir.join("subfinder");
        std::fs::write(&executable, b"#!/bin/sh\n").unwrap();

        assert_eq!(
            executable_candidate_path(&executable.display().to_string(), tools_dir).as_deref(),
            Some(executable.as_path())
        );
    }

    #[test]
    fn amass_engine_timeout_is_checked_empty_condition() {
        let task = ActiveTask {
            tool_id: "amass".into(),
            seed: "example.com".into(),
            args: Vec::new(),
            timeout_secs: 1,
        };

        assert!(is_amass_engine_timeout(
            &task,
            b"The Amass engine did not respond within the timeout period"
        ));
    }

    #[tokio::test]
    async fn active_task_missing_config_writes_failed_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let scope = ActiveScopeSet::from_targets(&[target("example.com")]);
        let task = ActiveTask {
            tool_id: "subfinder".into(),
            seed: "example.com".into(),
            args: vec!["-d".into(), "example.com".into()],
            timeout_secs: 1,
        };

        let result = run_active_task(
            &[],
            Path::new("/tmp/tools"),
            None,
            None,
            "run",
            dir.path(),
            &scope,
            task,
            None,
        )
        .await
        .unwrap();
        let manifest_path = dir.path().join("subfinder-example.com/manifest.json");
        let manifest: ReconTaskManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();

        assert_eq!(result.records.len(), 0);
        assert_eq!(result.errors[0].code, "active_tool_config_missing");
        assert_eq!(manifest.status, ReconTaskStatus::Failed);
        assert_eq!(manifest.source_id, "subfinder");
        assert_eq!(manifest.errors[0].code, "active_tool_config_missing");
        assert!(result
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "task_manifest"));
    }
}
