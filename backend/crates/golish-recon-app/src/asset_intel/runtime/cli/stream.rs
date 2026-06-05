//! Stream + artifact helpers for the cli_json provider runner: the shared
//! accumulator, stdout-line normalization, and `out_dir` artifact scanning.

use super::super::super::*;
use crate::organization_recon::artifacts::decode_utf8_clean;
use crate::organization_recon::ReconTaskError;

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
    pub(crate) candidates: TokioMutex<OrganizationCandidates>,
    /// Profile field entries lifted out of the same raw JSON documents.
    /// Stored separately from candidates because they target the master
    /// record (credit_code / industry / contacts / intel keys), not the
    /// review queue. The hydrate top-level merges these into a single
    /// `OrganizationProfilePatch` after the provider finishes.
    pub(crate) profile_entries: TokioMutex<Vec<ProfileFieldEntry>>,
    pub(crate) progress_buffer: TokioMutex<String>,
    pub(crate) stdout_raw: TokioMutex<Vec<u8>>,
    pub(crate) stderr_raw: TokioMutex<Vec<u8>>,
    pub(crate) diagnostics: TokioMutex<Vec<ReconTaskError>>,
    pub(crate) cancel: AtomicBool,
}

impl CliJsonStreamShared {
    pub(crate) fn new() -> Self {
        Self {
            candidates: TokioMutex::new(OrganizationCandidates::default()),
            profile_entries: TokioMutex::new(Vec::new()),
            progress_buffer: TokioMutex::new(String::new()),
            stdout_raw: TokioMutex::new(Vec::new()),
            stderr_raw: TokioMutex::new(Vec::new()),
            diagnostics: TokioMutex::new(Vec::new()),
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
#[allow(clippy::too_many_arguments)]
pub(crate) async fn scan_new_artifacts(
    out_dir: &Path,
    provider_id: &str,
    run_id: &str,
    normalize: &golish_pentest::models::AssetIntelNormalizeConfig,
    seen: &mut HashSet<PathBuf>,
    shared: &CliJsonStreamShared,
    sink: Option<&EventEmitterHandle>,
    record_errors: bool,
) -> Result<(), GolishError> {
    let mut files = Vec::new();
    collect_json_files(out_dir, &mut files)?;
    files.sort();
    for path in files {
        if !seen.insert(path.clone()) {
            continue;
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::debug!(
                    provider = %provider_id,
                    run_id,
                    artifact = %path.display(),
                    error = %err,
                    "asset_intel cli_json artifact read failed (skipping)"
                );
                if record_errors {
                    shared.diagnostics.lock().await.push(ReconTaskError::new(
                        "artifact_read_error",
                        format!("cannot read artifact '{}': {err}", path.display()),
                    ));
                }
                continue;
            }
        };
        let raw = match decode_utf8_clean(&bytes) {
            Ok(raw) => raw,
            Err(error) => {
                tracing::debug!(
                    provider = %provider_id,
                    run_id,
                    artifact = %path.display(),
                    error = %error.message,
                    "asset_intel cli_json artifact decode failed (skipping)"
                );
                if record_errors {
                    shared.diagnostics.lock().await.push(error);
                }
                continue;
            }
        };
        let Some((next, profile)) = normalize_json_document(provider_id, run_id, normalize, &raw)
        else {
            if record_errors {
                shared.diagnostics.lock().await.push(ReconTaskError::new(
                    "artifact_parse_error",
                    format!("cannot normalize artifact '{}'", path.display()),
                ));
            }
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
    golish_core::time::now_ms()
}
