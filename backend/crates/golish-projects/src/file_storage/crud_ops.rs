//! File save operations for captured JS, HTML, HTTP, tool output,
//! evidence, analysis reports, scripts, and host info.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, SystemTime};

#[cfg(test)]
type ReportFsAfterParentOpenHook = Box<dyn FnOnce() + Send + 'static>;

#[cfg(test)]
static REPORT_FS_AFTER_PARENT_OPEN_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<ReportFsAfterParentOpenHook>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
static REPORT_FS_AFTER_ROOT_CHECK_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<ReportFsAfterParentOpenHook>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn install_report_fs_after_parent_open_hook(hook: ReportFsAfterParentOpenHook) {
    *REPORT_FS_AFTER_PARENT_OPEN_HOOK
        .get_or_init(Default::default)
        .lock()
        .expect("report filesystem test hook mutex poisoned") = Some(hook);
}

#[cfg(test)]
fn install_report_fs_after_root_check_hook(hook: ReportFsAfterParentOpenHook) {
    *REPORT_FS_AFTER_ROOT_CHECK_HOOK
        .get_or_init(Default::default)
        .lock()
        .expect("report root test hook mutex poisoned") = Some(hook);
}

#[cfg(test)]
pub(super) fn run_report_fs_after_parent_open_hook() {
    let hook = REPORT_FS_AFTER_PARENT_OPEN_HOOK
        .get_or_init(Default::default)
        .lock()
        .expect("report filesystem test hook mutex poisoned")
        .take();
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
pub(super) fn run_report_fs_after_root_check_hook() {
    let hook = REPORT_FS_AFTER_ROOT_CHECK_HOOK
        .get_or_init(Default::default)
        .lock()
        .expect("report root test hook mutex poisoned")
        .take();
    if let Some(hook) = hook {
        hook();
    }
}

use super::{
    analysis_dir, captures_dir, evidence_dir, host_info_dir, scripts_dir, tool_output_dir,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportArtifactFormat {
    Markdown,
    Json,
}

impl ReportArtifactFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Json => "json",
        }
    }

    pub(super) const fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Json => "json",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StagedReportArtifact {
    pub revision_id: String,
    pub format: ReportArtifactFormat,
    pub staging_key: String,
    pub sha256: String,
    pub byte_len: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoredReportArtifact {
    pub format: ReportArtifactFormat,
    pub content_key: String,
    pub storage_path: String,
    pub sha256: String,
    pub byte_len: u64,
}

/// A promoted artifact plus the per-content publication reservation. Dropping
/// this value releases the cross-process advisory lock, so callers must retain
/// it until the database attachment transaction has completed.
pub struct ReservedReportArtifact {
    artifact: StoredReportArtifact,
    #[cfg(unix)]
    _reservation: super::report_artifacts_unix::ContentKeyReservation,
    #[cfg(windows)]
    _reservation: super::report_artifacts_windows::ContentKeyReservation,
}

impl ReservedReportArtifact {
    #[cfg(unix)]
    pub(super) fn new(
        artifact: StoredReportArtifact,
        reservation: super::report_artifacts_unix::ContentKeyReservation,
    ) -> Self {
        Self {
            artifact,
            _reservation: reservation,
        }
    }

    #[cfg(windows)]
    pub(super) fn new(
        artifact: StoredReportArtifact,
        reservation: super::report_artifacts_windows::ContentKeyReservation,
    ) -> Self {
        Self {
            artifact,
            _reservation: reservation,
        }
    }

    pub fn artifact(&self) -> &StoredReportArtifact {
        &self.artifact
    }
}

impl std::ops::Deref for ReservedReportArtifact {
    type Target = StoredReportArtifact;

    fn deref(&self) -> &Self::Target {
        &self.artifact
    }
}

impl std::fmt::Debug for ReservedReportArtifact {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("ReservedReportArtifact")
            .field(&self.artifact)
            .finish()
    }
}

impl PartialEq for ReservedReportArtifact {
    fn eq(&self, other: &Self) -> bool {
        self.artifact == other.artifact
    }
}

impl Eq for ReservedReportArtifact {}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct ReportArtifactGcOutcome {
    pub deleted_staging: u64,
    pub deleted_blobs: u64,
}

fn sha256_prefix(content: &[u8]) -> String {
    let hash = Sha256::digest(content);
    let hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
    hex[..8].to_string()
}

fn hashed_filename(original_name: &str, content: &[u8]) -> String {
    let prefix = sha256_prefix(content);
    format!("{}_{}", prefix, sanitize_filename(original_name))
}

pub(super) fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c => c,
        })
        .collect()
}

pub fn sanitize_report_path_component(name: &str) -> Result<String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        anyhow::bail!("report_revision_id_invalid");
    }
    Ok(name.to_string())
}

pub(super) fn sha256_hex(content: &[u8]) -> String {
    Sha256::digest(content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Write a server-named report artifact into project-local staging. The caller
/// supplies bytes only; the closed format and validated revision component own
/// every path segment.
pub async fn stage_report_artifact(
    project_root: &Path,
    revision_id: &str,
    format: ReportArtifactFormat,
    bytes: &[u8],
) -> Result<StagedReportArtifact> {
    #[cfg(unix)]
    {
        return super::report_artifacts_unix::stage(project_root, revision_id, format, bytes).await;
    }
    #[cfg(windows)]
    {
        return super::report_artifacts_windows::stage(project_root, revision_id, format, bytes)
            .await;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (project_root, revision_id, format, bytes);
        anyhow::bail!("report_artifact_storage_unsupported_os")
    }
}

/// Atomically publish a staged artifact with a hard-link put-if-absent. Since
/// staging and blobs share one project filesystem, no reader can observe a
/// partially written final blob and an existing content key is never replaced.
pub async fn promote_report_artifact(
    project_root: &Path,
    staged: &StagedReportArtifact,
) -> Result<ReservedReportArtifact> {
    #[cfg(unix)]
    {
        return super::report_artifacts_unix::promote(project_root, staged).await;
    }
    #[cfg(windows)]
    {
        return super::report_artifacts_windows::promote(project_root, staged).await;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (project_root, staged);
        anyhow::bail!("report_artifact_storage_unsupported_os")
    }
}

pub async fn verify_report_artifact(
    project_root: &Path,
    artifact: &StoredReportArtifact,
) -> Result<bool> {
    #[cfg(unix)]
    {
        return super::report_artifacts_unix::verify(project_root, artifact).await;
    }
    #[cfg(windows)]
    {
        return super::report_artifacts_windows::verify(project_root, artifact).await;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (project_root, artifact);
        anyhow::bail!("report_artifact_storage_unsupported_os")
    }
}

pub async fn discard_staged_report_artifact(
    project_root: &Path,
    staged: &StagedReportArtifact,
) -> Result<()> {
    #[cfg(unix)]
    {
        return super::report_artifacts_unix::discard(project_root, staged).await;
    }
    #[cfg(windows)]
    {
        return super::report_artifacts_windows::discard(project_root, staged).await;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (project_root, staged);
        anyhow::bail!("report_artifact_storage_unsupported_os")
    }
}

/// Remove only grace-expired staging files and unreferenced final blobs.
/// Database references are supplied by the composition-root GC worker; this
/// filesystem seam never guesses publication state.
pub async fn gc_report_artifacts(
    project_root: &Path,
    now: SystemTime,
    grace_period: Duration,
    referenced_content_keys: &BTreeSet<String>,
) -> Result<ReportArtifactGcOutcome> {
    #[cfg(unix)]
    {
        return super::report_artifacts_unix::gc(
            project_root,
            now,
            grace_period,
            referenced_content_keys,
        )
        .await;
    }
    #[cfg(windows)]
    {
        return super::report_artifacts_windows::gc(
            project_root,
            now,
            grace_period,
            referenced_content_keys,
        )
        .await;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (project_root, now, grace_period, referenced_content_keys);
        anyhow::bail!("report_artifact_storage_unsupported_os")
    }
}

fn url_path_slug(url_path: &str) -> String {
    url_path
        .trim_start_matches('/')
        .replace('/', "-")
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c => c,
        })
        .collect::<String>()
        .chars()
        .take(100)
        .collect()
}

/// Save a captured JS file. Returns the relative path from project root.
pub async fn save_js_capture(
    project_root: &Path,
    host: &str,
    port: u16,
    filename: &str,
    content: &[u8],
    url_path: Option<&str>,
) -> Result<String> {
    let base = captures_dir(project_root, host, port).join("js");

    let dir = if let Some(url_p) = url_path {
        let trimmed = url_p.trim_start_matches('/');
        if let Some(parent) = std::path::Path::new(trimmed).parent() {
            if !parent.as_os_str().is_empty() {
                let safe_parent = parent
                    .to_string_lossy()
                    .replace("..", "_")
                    .replace(':', "_");
                base.join(safe_parent)
            } else {
                base
            }
        } else {
            base
        }
    } else {
        base
    };

    tokio::fs::create_dir_all(&dir).await?;

    let safe_name = hashed_filename(filename, content);
    let full_path = dir.join(&safe_name);
    tokio::fs::write(&full_path, content).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();

    tracing::debug!("[file-storage] Saved JS capture: {}", rel);
    Ok(rel)
}

/// Save a captured HTML file. Returns the relative path from project root.
pub async fn save_html_capture(
    project_root: &Path,
    host: &str,
    port: u16,
    url_path: &str,
    content: &[u8],
) -> Result<String> {
    let dir = captures_dir(project_root, host, port).join("html");
    tokio::fs::create_dir_all(&dir).await?;

    let slug = url_path_slug(url_path);
    let safe_name = format!("{}_{}.html", sha256_prefix(content), slug);
    let full_path = dir.join(&safe_name);
    tokio::fs::write(&full_path, content).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

/// Save an HTTP request/response pair as JSON. Returns the relative path.
pub async fn save_http_capture(
    project_root: &Path,
    host: &str,
    port: u16,
    method: &str,
    url_path: &str,
    content: &[u8],
) -> Result<String> {
    let dir = captures_dir(project_root, host, port).join("http");
    tokio::fs::create_dir_all(&dir).await?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let slug = url_path_slug(url_path);
    let filename = format!(
        "{}_{}{}.json",
        timestamp,
        method,
        if slug.is_empty() {
            "root".to_string()
        } else {
            format!("_{}", slug)
        }
    );
    let full_path = dir.join(&filename);
    tokio::fs::write(&full_path, content).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

/// Save tool output. Returns the relative path.
pub async fn save_tool_output(
    project_root: &Path,
    tool_name: &str,
    target_slug: &str,
    extension: &str,
    content: &[u8],
) -> Result<String> {
    let dir = tool_output_dir(project_root, tool_name);
    tokio::fs::create_dir_all(&dir).await?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let safe_target = sanitize_filename(target_slug);
    let filename = format!("{}_{}.{}", timestamp, safe_target, extension);
    let full_path = dir.join(&filename);
    tokio::fs::write(&full_path, content).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    tracing::debug!("[file-storage] Saved tool output: {}", rel);
    Ok(rel)
}

/// Save an evidence file for a finding. Returns the relative path.
pub async fn save_evidence(
    project_root: &Path,
    finding_id: &str,
    filename: &str,
    content: &[u8],
) -> Result<String> {
    let dir = evidence_dir(project_root, finding_id);
    tokio::fs::create_dir_all(&dir).await?;

    let safe_name = sanitize_filename(filename);
    let full_path = dir.join(&safe_name);
    tokio::fs::write(&full_path, content).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

/// Save an AI analysis report. Returns the relative path.
pub async fn save_analysis_report(
    project_root: &Path,
    host: &str,
    analysis_type: &str,
    content: &str,
) -> Result<String> {
    let dir = analysis_dir(project_root, host);
    tokio::fs::create_dir_all(&dir).await?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let filename = format!("{}_{}.md", sanitize_filename(analysis_type), timestamp);
    let full_path = dir.join(&filename);
    tokio::fs::write(&full_path, content.as_bytes()).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

/// Save a script to the appropriate category directory. Returns the relative path.
pub async fn save_script(
    project_root: &Path,
    category: &str,
    filename: &str,
    content: &str,
) -> Result<String> {
    let dir = scripts_dir(project_root, category);
    tokio::fs::create_dir_all(&dir).await?;

    let safe_name = sanitize_filename(filename);
    let full_path = dir.join(&safe_name);
    tokio::fs::write(&full_path, content.as_bytes()).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

/// Save host-level info (DNS, WHOIS, etc.). Returns the relative path.
pub async fn save_host_info(
    project_root: &Path,
    host: &str,
    filename: &str,
    content: &[u8],
) -> Result<String> {
    let dir = host_info_dir(project_root, host);
    tokio::fs::create_dir_all(&dir).await?;

    let safe_name = sanitize_filename(filename);
    let full_path = dir.join(&safe_name);
    tokio::fs::write(&full_path, content).await?;

    let rel = full_path
        .strip_prefix(project_root)
        .unwrap_or(&full_path)
        .to_string_lossy()
        .to_string();
    Ok(rel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    #[cfg(unix)]
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    fn canonical_temp_root(root: &tempfile::TempDir) -> PathBuf {
        std::fs::canonicalize(root.path()).expect("canonical temporary root")
    }

    #[cfg(unix)]
    fn backdate_file(path: &Path, age: Duration) {
        use std::os::fd::AsRawFd;

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .expect("open timestamp fixture");
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("time after epoch")
            .checked_sub(age)
            .expect("fixture age before now");
        let times = [
            libc::timespec {
                tv_sec: i64::try_from(timestamp.as_secs()).expect("fixture seconds"),
                tv_nsec: i64::from(timestamp.subsec_nanos()),
            },
            libc::timespec {
                tv_sec: i64::try_from(timestamp.as_secs()).expect("fixture seconds"),
                tv_nsec: i64::from(timestamp.subsec_nanos()),
            },
        ];
        assert_eq!(
            unsafe { libc::futimens(file.as_raw_fd(), times.as_ptr()) },
            0
        );
        file.sync_all().expect("sync timestamp fixture");
    }

    #[test]
    fn test_sha256_prefix() {
        let content = b"hello world";
        let prefix = sha256_prefix(content);
        assert_eq!(prefix.len(), 8);
        assert_eq!(prefix, "b94d27b9");
    }

    #[test]
    fn test_hashed_filename() {
        let name = hashed_filename("app.js", b"console.log('hi')");
        assert!(name.ends_with("_app.js"));
        assert_eq!(name.len(), 8 + 1 + "app.js".len());
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("file.txt"), "file.txt");
        assert_eq!(sanitize_filename("path/to/file"), "path_to_file");
        assert_eq!(sanitize_filename("file<>:\""), "file____");
    }

    #[test]
    fn test_url_path_slug() {
        assert_eq!(url_path_slug("/api/v1/users"), "api-v1-users");
        assert_eq!(url_path_slug("/"), "");
        assert_eq!(url_path_slug("/login"), "login");
    }

    #[tokio::test]
    async fn report_artifacts_are_content_addressed_and_promoted_without_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let project_root = canonical_temp_root(&root);
        let first = stage_report_artifact(
            &project_root,
            "019f0000-0000-7000-8000-000000000001",
            ReportArtifactFormat::Markdown,
            b"# report\n",
        )
        .await
        .unwrap();
        let second = stage_report_artifact(
            &project_root,
            "019f0000-0000-7000-8000-000000000002",
            ReportArtifactFormat::Markdown,
            b"# report\n",
        )
        .await
        .unwrap();

        let promoted_first = promote_report_artifact(&project_root, &first)
            .await
            .unwrap();
        let promoted_first_artifact = promoted_first.artifact().clone();
        drop(promoted_first);
        let promoted_second = promote_report_artifact(&project_root, &second)
            .await
            .unwrap();
        let promoted_second_artifact = promoted_second.artifact().clone();
        drop(promoted_second);

        assert_eq!(promoted_first_artifact, promoted_second_artifact);
        assert!(
            verify_report_artifact(&project_root, &promoted_first_artifact)
                .await
                .unwrap()
        );
        assert!(promoted_first_artifact
            .storage_path
            .starts_with(".golish/reports/blobs/"));
        assert!(!project_root.join(&first.staging_key).exists());
        assert!(!project_root.join(&second.staging_key).exists());
    }

    #[tokio::test]
    async fn report_artifact_keys_cannot_escape_project_root() {
        let root = tempfile::tempdir().unwrap();
        let project_root = canonical_temp_root(&root);
        let error = stage_report_artifact(
            &project_root,
            "../../escape",
            ReportArtifactFormat::Json,
            br#"{}"#,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("report_revision_id_invalid"));
        assert!(!project_root.join("escape").exists());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn report_stage_rejects_parent_swap_after_directory_open_without_external_writes() {
        use std::os::unix::fs::symlink;
        use std::sync::mpsc;

        let project = tempfile::tempdir().expect("temporary project root");
        let project_root = canonical_temp_root(&project);
        let external = tempfile::tempdir().expect("temporary external root");
        let revision_id = "019f0000-0000-7000-8000-000000000007";
        let bytes = b"# race-safe report\n";
        let sha256 = sha256_hex(bytes);
        let external_sentinel = external.path().join("sentinel.txt");
        tokio::fs::write(&external_sentinel, b"outside must remain unchanged")
            .await
            .expect("write external sentinel");

        let (parent_opened_tx, parent_opened_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        install_report_fs_after_parent_open_hook(Box::new(move || {
            parent_opened_tx
                .send(())
                .expect("signal that the checked parent was opened");
            release_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("release staged writer after parent swap");
        }));

        let stage_root = project_root.clone();
        let stage = tokio::spawn(async move {
            stage_report_artifact(
                &stage_root,
                revision_id,
                ReportArtifactFormat::Markdown,
                bytes,
            )
            .await
        });
        tokio::task::spawn_blocking(move || {
            parent_opened_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("stage reached the post-open barrier")
        })
        .await
        .expect("join barrier waiter");

        let staging_root = project_root.join(".golish/reports/.staging");
        let checked_parent = staging_root.join(revision_id);
        let detached_parent = staging_root.join(format!("{revision_id}.detached"));
        tokio::fs::rename(&checked_parent, &detached_parent)
            .await
            .expect("detach the already checked directory");
        symlink(external.path(), &checked_parent)
            .expect("replace the checked parent with an external symlink");
        release_tx.send(()).expect("release staged writer");

        let result = stage.await.expect("join staged writer");
        assert!(
            result.is_err(),
            "a changed project-relative parent binding must fail closed"
        );
        assert_eq!(
            tokio::fs::read(&external_sentinel)
                .await
                .expect("external sentinel remains readable"),
            b"outside must remain unchanged"
        );
        assert!(
            !external.path().join(format!("{sha256}.md")).exists(),
            "the replaced parent must never receive the artifact"
        );
        assert!(
            !detached_parent.join(format!("{sha256}.md")).exists(),
            "the operation must refuse the changed binding instead of silently using the old inode"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn report_stage_rejects_preexisting_symlink_project_root() {
        use std::os::unix::fs::symlink;

        let container = tempfile::tempdir().expect("temporary root container");
        let external = tempfile::tempdir().expect("temporary external root");
        let project_link = container.path().join("project-link");
        symlink(external.path(), &project_link).expect("create symlink project root");
        let sentinel = external.path().join("sentinel.txt");
        tokio::fs::write(&sentinel, b"outside")
            .await
            .expect("write external sentinel");

        assert!(
            stage_report_artifact(
                &project_link,
                "019f0000-0000-7000-8000-000000000008",
                ReportArtifactFormat::Markdown,
                b"# must not land outside\n",
            )
            .await
            .is_err(),
            "a preexisting symlink project root must fail closed"
        );
        assert_eq!(tokio::fs::read(&sentinel).await.unwrap(), b"outside");
        assert!(!external.path().join(".golish").exists());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn report_stage_rejects_project_root_swap_after_check_without_external_writes() {
        use std::os::unix::fs::symlink;
        use std::sync::mpsc;

        let container = tempfile::tempdir().expect("temporary root container");
        let project = container.path().join("project");
        tokio::fs::create_dir(&project)
            .await
            .expect("create project root");
        let project = tokio::fs::canonicalize(&project)
            .await
            .expect("canonical project root fixture");
        let detached = project.with_extension("detached");
        let external = tempfile::tempdir().expect("temporary external root");
        let sentinel = external.path().join("sentinel.txt");
        tokio::fs::write(&sentinel, b"outside")
            .await
            .expect("write external sentinel");
        let (root_checked_tx, root_checked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        install_report_fs_after_root_check_hook(Box::new(move || {
            root_checked_tx.send(()).expect("signal root check");
            release_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("release root opener");
        }));

        let stage_root = project.clone();
        let stage = tokio::spawn(async move {
            stage_report_artifact(
                &stage_root,
                "019f0000-0000-7000-8000-000000000009",
                ReportArtifactFormat::Json,
                br#"{"outside":false}"#,
            )
            .await
        });
        tokio::task::spawn_blocking(move || {
            root_checked_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("stage reached root check barrier")
        })
        .await
        .expect("join root barrier waiter");
        tokio::fs::rename(&project, &detached)
            .await
            .expect("detach checked project root");
        symlink(external.path(), &project).expect("replace project root with symlink");
        release_tx.send(()).expect("release root opener");

        let result = stage.await.expect("join stage task");
        assert!(result.is_err(), "changed root binding must fail closed");
        assert_eq!(tokio::fs::read(&sentinel).await.unwrap(), b"outside");
        assert!(
            !external.path().join(".golish").exists(),
            "the substituted project root must receive no storage mutation"
        );
        assert!(
            !detached.join(".golish").exists(),
            "the detached original root must receive no storage mutation"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[serial_test::serial]
    async fn report_stage_replay_refreshes_grace_while_gc_waits_on_content_lock() {
        use std::sync::mpsc;

        let project = tempfile::tempdir().expect("temporary project root");
        let project_root = canonical_temp_root(&project);
        let revision_id = "019f0000-0000-7000-8000-000000000010";
        let bytes = b"# retry staging lease\n";
        let staged = stage_report_artifact(
            &project_root,
            revision_id,
            ReportArtifactFormat::Markdown,
            bytes,
        )
        .await
        .expect("initial stage");
        let staging_path = project_root.join(&staged.staging_key);
        let grace = Duration::from_secs(60 * 60);
        backdate_file(&staging_path, grace + Duration::from_secs(60));

        let (parent_opened_tx, parent_opened_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        install_report_fs_after_parent_open_hook(Box::new(move || {
            parent_opened_tx.send(()).expect("signal replay open");
            release_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("release staging replay");
        }));
        let replay_root = project_root.clone();
        let replay = tokio::spawn(async move {
            stage_report_artifact(
                &replay_root,
                revision_id,
                ReportArtifactFormat::Markdown,
                bytes,
            )
            .await
        });
        tokio::task::spawn_blocking(move || {
            parent_opened_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("replay reached post-open barrier")
        })
        .await
        .expect("join replay barrier waiter");

        let gc_root = project_root.clone();
        let mut gc = tokio::spawn(async move {
            gc_report_artifacts(&gc_root, SystemTime::now(), grace, &BTreeSet::new()).await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(150), &mut gc)
                .await
                .is_err(),
            "GC must wait while a staging replay owns the content-key lock"
        );
        release_tx.send(()).expect("release staging replay");
        let replayed = replay
            .await
            .expect("join staging replay")
            .expect("staging replay succeeds");
        let gc_outcome = gc
            .await
            .expect("join staging GC")
            .expect("staging GC succeeds");
        assert_eq!(replayed, staged);
        assert_eq!(gc_outcome.deleted_staging, 0);
        assert!(
            staging_path.exists(),
            "fresh replayed staging must survive GC"
        );
    }

    #[tokio::test]
    async fn report_artifact_gc_keeps_references_and_removes_grace_expired_orphans() {
        let root = tempfile::tempdir().unwrap();
        let project_root = canonical_temp_root(&root);
        let referenced_reservation = promote_report_artifact(
            &project_root,
            &stage_report_artifact(
                &project_root,
                "019f0000-0000-7000-8000-000000000003",
                ReportArtifactFormat::Json,
                br#"{"kept":true}"#,
            )
            .await
            .unwrap(),
        )
        .await
        .unwrap();
        let referenced = referenced_reservation.artifact().clone();
        drop(referenced_reservation);
        let orphan_reservation = promote_report_artifact(
            &project_root,
            &stage_report_artifact(
                &project_root,
                "019f0000-0000-7000-8000-000000000004",
                ReportArtifactFormat::Json,
                br#"{"orphan":true}"#,
            )
            .await
            .unwrap(),
        )
        .await
        .unwrap();
        let orphan = orphan_reservation.artifact().clone();
        drop(orphan_reservation);

        let outcome = gc_report_artifacts(
            &project_root,
            SystemTime::now() + Duration::from_secs(1),
            Duration::ZERO,
            &BTreeSet::from([referenced.content_key.clone()]),
        )
        .await
        .unwrap();

        assert_eq!(outcome.deleted_blobs, 1);
        assert!(project_root.join(&referenced.storage_path).exists());
        assert!(!project_root.join(&orphan.storage_path).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn report_artifact_operations_reject_symlinked_reports_root_without_external_changes() {
        use std::os::unix::fs::symlink;

        let project = tempfile::tempdir().expect("temporary project root");
        let project_root = canonical_temp_root(&project);
        let external = tempfile::tempdir().expect("temporary external report root");
        tokio::fs::create_dir_all(project_root.join(".golish"))
            .await
            .expect("create project metadata root");
        symlink(external.path(), project_root.join(".golish/reports"))
            .expect("link reports root outside project");

        let revision_id = "019f0000-0000-7000-8000-000000000005";
        let bytes = b"# external sentinel\n";
        let sha256 = sha256_hex(bytes);
        let staging_path = external
            .path()
            .join(".staging")
            .join(revision_id)
            .join(format!("{sha256}.md"));
        let blob_path = external
            .path()
            .join("blobs")
            .join("sha256")
            .join(format!("{sha256}.md"));
        tokio::fs::create_dir_all(staging_path.parent().expect("staging parent"))
            .await
            .expect("create external staging parent");
        tokio::fs::create_dir_all(blob_path.parent().expect("blob parent"))
            .await
            .expect("create external blob parent");
        tokio::fs::write(&staging_path, bytes)
            .await
            .expect("write external staged sentinel");
        tokio::fs::write(&blob_path, bytes)
            .await
            .expect("write external blob sentinel");

        let staged = StagedReportArtifact {
            revision_id: revision_id.to_string(),
            format: ReportArtifactFormat::Markdown,
            staging_key: format!(".golish/reports/.staging/{revision_id}/{sha256}.md"),
            sha256: sha256.clone(),
            byte_len: u64::try_from(bytes.len()).expect("sentinel byte length"),
        };
        let artifact = StoredReportArtifact {
            format: ReportArtifactFormat::Markdown,
            content_key: format!("sha256/{sha256}.md"),
            storage_path: format!(".golish/reports/blobs/sha256/{sha256}.md"),
            sha256,
            byte_len: u64::try_from(bytes.len()).expect("sentinel byte length"),
        };

        let stage_result = stage_report_artifact(
            &project_root,
            revision_id,
            ReportArtifactFormat::Markdown,
            bytes,
        )
        .await;
        let promote_result = promote_report_artifact(&project_root, &staged).await;
        let verify_result = verify_report_artifact(&project_root, &artifact).await;
        let discard_result = discard_staged_report_artifact(&project_root, &staged).await;
        let gc_result = gc_report_artifacts(
            &project_root,
            SystemTime::now() + Duration::from_secs(1),
            Duration::ZERO,
            &BTreeSet::new(),
        )
        .await;

        assert!(stage_result.is_err(), "stage must reject reports symlink");
        assert!(
            promote_result.is_err(),
            "promote must reject reports symlink"
        );
        assert!(
            verify_result.is_err(),
            "read/verify must reject reports symlink"
        );
        assert!(
            discard_result.is_err(),
            "discard must reject reports symlink"
        );
        assert!(gc_result.is_err(), "GC must reject reports symlink");
        assert_eq!(
            tokio::fs::read(&staging_path)
                .await
                .expect("external staged sentinel remains"),
            bytes
        );
        assert_eq!(
            tokio::fs::read(&blob_path)
                .await
                .expect("external blob sentinel remains"),
            bytes
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn report_artifact_operations_reject_symlinked_file_destinations() {
        use std::os::unix::fs::symlink;

        let project = tempfile::tempdir().expect("temporary project root");
        let project_root = canonical_temp_root(&project);
        let external = tempfile::tempdir().expect("temporary external root");
        let revision_id = "019f0000-0000-7000-8000-000000000006";
        let bytes = b"external report bytes\n";
        let sha256 = sha256_hex(bytes);
        let external_file = external.path().join("report.md");
        tokio::fs::write(&external_file, bytes)
            .await
            .expect("write external report sentinel");

        let staging_parent = project_root
            .join(".golish/reports/.staging")
            .join(revision_id);
        let blob_parent = project_root.join(".golish/reports/blobs/sha256");
        tokio::fs::create_dir_all(&staging_parent)
            .await
            .expect("create staging parent");
        tokio::fs::create_dir_all(&blob_parent)
            .await
            .expect("create blob parent");
        let staging_path = staging_parent.join(format!("{sha256}.md"));
        let blob_path = blob_parent.join(format!("{sha256}.md"));
        symlink(&external_file, &staging_path).expect("link staged destination externally");
        symlink(&external_file, &blob_path).expect("link blob destination externally");

        let staged = StagedReportArtifact {
            revision_id: revision_id.to_string(),
            format: ReportArtifactFormat::Markdown,
            staging_key: format!(".golish/reports/.staging/{revision_id}/{sha256}.md"),
            sha256: sha256.clone(),
            byte_len: u64::try_from(bytes.len()).expect("sentinel byte length"),
        };
        let artifact = StoredReportArtifact {
            format: ReportArtifactFormat::Markdown,
            content_key: format!("sha256/{sha256}.md"),
            storage_path: format!(".golish/reports/blobs/sha256/{sha256}.md"),
            sha256,
            byte_len: u64::try_from(bytes.len()).expect("sentinel byte length"),
        };

        assert!(
            stage_report_artifact(
                &project_root,
                revision_id,
                ReportArtifactFormat::Markdown,
                bytes,
            )
            .await
            .is_err(),
            "stage must reject a symlink destination"
        );
        assert!(
            promote_report_artifact(&project_root, &staged)
                .await
                .is_err(),
            "promote must reject a symlink source"
        );
        assert!(
            verify_report_artifact(&project_root, &artifact)
                .await
                .is_err(),
            "read/verify must reject a symlink destination"
        );
        assert!(
            discard_staged_report_artifact(&project_root, &staged)
                .await
                .is_err(),
            "discard must reject a symlink destination"
        );
        assert!(
            gc_report_artifacts(
                &project_root,
                SystemTime::now() + Duration::from_secs(1),
                Duration::ZERO,
                &BTreeSet::new(),
            )
            .await
            .is_err(),
            "GC must reject symlink entries"
        );
        assert_eq!(
            tokio::fs::read(&external_file)
                .await
                .expect("external sentinel remains"),
            bytes
        );
        assert!(tokio::fs::symlink_metadata(&staging_path)
            .await
            .expect("staging symlink remains")
            .file_type()
            .is_symlink());
        assert!(tokio::fs::symlink_metadata(&blob_path)
            .await
            .expect("blob symlink remains")
            .file_type()
            .is_symlink());
    }
}
