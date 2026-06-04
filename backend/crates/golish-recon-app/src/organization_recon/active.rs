use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use golish_app_core::GolishError;
use golish_pentest::models::ToolConfig;
use serde_json::Value;
use tokio::process::Command;

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

#[derive(Debug)]
pub(crate) struct ActiveCollectionOutcome {
    pub status: ReconTaskStatus,
    pub record_count: usize,
    pub errors: Vec<ReconTaskError>,
    pub artifacts: Vec<ReconArtifactRef>,
}

#[derive(Debug, Clone)]
struct ActiveTask {
    tool_id: &'static str,
    seed: String,
    args: Vec<String>,
    timeout_secs: u64,
}

#[derive(Debug, Default)]
struct ActiveScopeSet {
    roots: BTreeSet<String>,
    hosts: BTreeSet<String>,
}

pub(crate) async fn run_active_collection(
    scan_tools: &[ToolConfig],
    tools_dir: &Path,
    run_id: &str,
    active_targets: &[Target],
    active_dir: &Path,
) -> Result<ActiveCollectionOutcome, GolishError> {
    let scope = ActiveScopeSet::from_targets(active_targets);
    let scope_bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "runId": run_id,
        "rootDomains": scope.roots,
        "hosts": scope.hosts,
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
        match run_active_task(scan_tools, tools_dir, run_id, active_dir, &scope, task).await {
            Ok(result) => {
                artifacts.extend(result.artifacts);
                errors.extend(result.errors);
                records.extend(result.records);
            }
            Err(error) => errors.push(ReconTaskError::new(
                "active_task_failed",
                format!("active task failed: {error}"),
            )),
        }
    }

    let records = merge_normalized_records(records);
    let records_artifact = write_records_jsonl(active_dir, &records)?;
    artifacts.push(records_artifact);

    let status = if records.is_empty() && !errors.is_empty() {
        ReconTaskStatus::Failed
    } else if records.is_empty() {
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

async fn run_active_task(
    scan_tools: &[ToolConfig],
    tools_dir: &Path,
    run_id: &str,
    active_dir: &Path,
    scope: &ActiveScopeSet,
    task: ActiveTask,
) -> Result<ActiveTaskResult, GolishError> {
    let task_dir = active_dir.join(safe_task_name(task.tool_id, &task.seed));
    let Some(exec) = golish_pentest::resolve_tool_executable(task.tool_id, scan_tools, tools_dir)
    else {
        let errors = vec![ReconTaskError::new(
            "active_tool_config_missing",
            format!("tool config '{}' not found", task.tool_id),
        )];
        let mut artifacts = Vec::new();
        write_active_task_manifest(
            &task_dir,
            run_id,
            &task,
            ReconTaskStatus::Failed,
            None,
            &mut artifacts,
            0,
            &errors,
        )?;
        return Ok(ActiveTaskResult {
            records: Vec::new(),
            errors,
            artifacts,
        });
    };

    let argv = serde_json::to_vec_pretty(&serde_json::json!({
        "toolId": task.tool_id,
        "executable": exec,
        "args": task.args,
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

    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let errors = vec![ReconTaskError::new(
                "active_tool_spawn_failed",
                format!("{} spawn failed: {error}", task.tool_id),
            )];
            write_active_task_manifest(
                &task_dir,
                run_id,
                &task,
                ReconTaskStatus::Failed,
                None,
                &mut artifacts,
                0,
                &errors,
            )?;
            return Ok(ActiveTaskResult {
                records: Vec::new(),
                errors,
                artifacts,
            });
        }
    };

    let timeout = Duration::from_secs(task.timeout_secs.clamp(1, 1800));
    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            let errors = vec![ReconTaskError::new(
                "active_tool_wait_failed",
                format!("{} wait failed: {error}", task.tool_id),
            )];
            write_active_task_manifest(
                &task_dir,
                run_id,
                &task,
                ReconTaskStatus::Failed,
                None,
                &mut artifacts,
                0,
                &errors,
            )?;
            return Ok(ActiveTaskResult {
                records: Vec::new(),
                errors,
                artifacts,
            });
        }
        Err(_) => {
            let errors = vec![ReconTaskError::new(
                "active_tool_timeout",
                format!("{} timed out after {}s", task.tool_id, task.timeout_secs),
            )];
            write_active_task_manifest(
                &task_dir,
                run_id,
                &task,
                ReconTaskStatus::Failed,
                None,
                &mut artifacts,
                0,
                &errors,
            )?;
            return Ok(ActiveTaskResult {
                records: Vec::new(),
                errors,
                artifacts,
            });
        }
    };

    let stdout_artifact = write_raw_bytes(&task_dir, "raw/stdout.log", &output.stdout, "stdout")?;
    let stderr_artifact = write_raw_bytes(&task_dir, "raw/stderr.log", &output.stderr, "stderr")?;
    let stdout_path = stdout_artifact.path.clone();
    artifacts.push(stdout_artifact);
    artifacts.push(stderr_artifact);

    let mut errors = Vec::new();
    if !output.status.success() {
        errors.push(ReconTaskError::new(
            "active_tool_nonzero_exit",
            format!(
                "{} exited with {:?}",
                task.tool_id,
                output.status.code().unwrap_or(-1)
            ),
        ));
    }

    let records = match decode_utf8_clean(&output.stdout) {
        Ok(stdout) => parse_records(run_id, &task, scope, &stdout, &stdout_path),
        Err(error) => {
            errors.push(error);
            Vec::new()
        }
    };
    let status = if !errors.is_empty() {
        ReconTaskStatus::Failed
    } else if records.is_empty() {
        ReconTaskStatus::CheckedEmpty
    } else {
        ReconTaskStatus::Completed
    };
    write_active_task_manifest(
        &task_dir,
        run_id,
        &task,
        status,
        output.status.code(),
        &mut artifacts,
        records.len(),
        &errors,
    )?;

    Ok(ActiveTaskResult {
        records,
        errors,
        artifacts,
    })
}

fn write_active_task_manifest(
    task_dir: &Path,
    run_id: &str,
    task: &ActiveTask,
    status: ReconTaskStatus,
    exit_code: Option<i32>,
    artifacts: &mut Vec<ReconArtifactRef>,
    record_count: usize,
    errors: &[ReconTaskError],
) -> Result<(), GolishError> {
    let mut manifest = ReconTaskManifest::new(
        run_id,
        safe_task_name(task.tool_id, &task.seed),
        "active_collection",
        task.tool_id,
    );
    manifest.status = status;
    manifest.exit_code = exit_code;
    manifest.artifacts = artifacts.clone();
    manifest.record_count = record_count;
    manifest.checked_empty = matches!(manifest.status, ReconTaskStatus::CheckedEmpty);
    manifest.errors = errors.to_vec();

    let manifest_path = write_json_manifest(task_dir, &manifest)?;
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
            tool_id: "subfinder",
            seed: root.clone(),
            args: vec!["-d".into(), root.clone(), "-silent".into()],
            timeout_secs: 900,
        });
        tasks.push(ActiveTask {
            tool_id: "amass",
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
            tool_id: "nmap",
            seed: host.clone(),
            args: vec![
                host.clone(),
                "--top-ports".into(),
                "100".into(),
                "-T3".into(),
            ],
            timeout_secs: 900,
        });
        tasks.push(ActiveTask {
            tool_id: "httpx",
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
    tasks
}

fn parse_records(
    run_id: &str,
    task: &ActiveTask,
    scope: &ActiveScopeSet,
    stdout: &str,
    raw_artifact_path: &str,
) -> Vec<NormalizedReconRecord> {
    match task.tool_id {
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
            task_id: safe_task_name(task.tool_id, &task.seed),
            raw_artifact_path: raw_artifact_path.into(),
        }],
    })
}

fn json_attrs(key: &str, value: &str) -> Value {
    serde_json::json!({ key: value })
}

impl ActiveScopeSet {
    fn from_targets(targets: &[Target]) -> Self {
        let mut scope = Self::default();
        for value in targets.iter().map(|target| target.value.trim()) {
            if value.is_empty() {
                continue;
            }
            if let Some(host) = host_from_target_value(value) {
                scope.hosts.insert(host.clone());
                if looks_like_domain(&host) {
                    scope.roots.insert(host);
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
            tool_id: "subfinder",
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
            tool_id: "httpx",
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

    #[tokio::test]
    async fn active_task_missing_config_writes_failed_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let scope = ActiveScopeSet::from_targets(&[target("example.com")]);
        let task = ActiveTask {
            tool_id: "subfinder",
            seed: "example.com".into(),
            args: vec!["-d".into(), "example.com".into()],
            timeout_secs: 1,
        };

        let result = run_active_task(
            &[],
            Path::new("/tmp/tools"),
            "run",
            dir.path(),
            &scope,
            task,
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
