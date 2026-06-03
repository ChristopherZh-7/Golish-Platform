//! Background job manager for long-running AI shell/pentest commands.
//!
//! Replaces the old "timeout → kill the process → report a timeout error"
//! behaviour with a Cursor-style "soft timeout → keep running in the
//! background" model:
//!
//! - A command is always spawned through [`BackgroundJobManager::spawn`], which
//!   captures stdout/stderr incrementally into a capped buffer and **does not**
//!   `kill_on_drop` — the child outlives the awaiting future.
//! - The caller (`run_shell_command_detail`) waits up to a *soft* timeout; if
//!   the command finishes in time it returns the full result as before, else it
//!   returns a `backgrounded` handle (`job_id`) and the command keeps running.
//! - The AI polls [`BackgroundJobManager::snapshot`] via the `check_job` tool.
//! - A *hard* limit watchdog kills runaway jobs so nothing leaks forever.
//!
//! Design: `docs/design/2026-06-03-background-tool-execution.md` (P1).

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;

use crate::pty_interactive::shell_command;

/// Cap each stream's retained buffer; keep the *tail* once exceeded so the most
/// recent output (usually the interesting part) survives.
const MAX_JOB_OUTPUT_BYTES: usize = 512 * 1024;
/// Soft-delete finished jobs once the map grows past this, so a long session
/// doesn't accumulate unbounded completed-job state.
const MAX_RETAINED_JOBS: usize = 128;

/// High-level lifecycle state of a background job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Done,
    Failed,
    Killed,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Killed => "killed",
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, JobStatus::Running)
    }
}

struct JobState {
    command: String,
    status: JobStatus,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    started_at: Instant,
    finished_at: Option<Instant>,
    kill: Arc<Notify>,
}

/// Immutable view of a job's current state, safe to hand back to callers.
#[derive(Debug, Clone)]
pub struct JobSnapshot {
    pub command: String,
    pub status: JobStatus,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub finished: bool,
}

/// Process-wide registry of background jobs.
pub struct BackgroundJobManager {
    jobs: Mutex<HashMap<String, Arc<Mutex<JobState>>>>,
}

impl Default for BackgroundJobManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Append `chunk` to `buf`, trimming from the front (on a char boundary) so the
/// retained size never exceeds `MAX_JOB_OUTPUT_BYTES`.
fn append_capped(buf: &mut String, chunk: &str) {
    buf.push_str(chunk);
    if buf.len() <= MAX_JOB_OUTPUT_BYTES {
        return;
    }
    let mut cut = buf.len() - MAX_JOB_OUTPUT_BYTES;
    while cut < buf.len() && !buf.is_char_boundary(cut) {
        cut += 1;
    }
    *buf = buf[cut..].to_string();
}

async fn pump<R>(mut reader: R, state: Arc<Mutex<JobState>>, is_stderr: bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]);
                let mut s = state.lock();
                if is_stderr {
                    append_capped(&mut s.stderr, &chunk);
                } else {
                    append_capped(&mut s.stdout, &chunk);
                }
            }
        }
    }
}

impl BackgroundJobManager {
    pub fn new() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
        }
    }

    /// Spawn `command` in the background. Returns immediately with a `job_id`.
    /// The child is reaped by an internal task; output streams into a capped
    /// buffer; the `hard_limit` watchdog kills it if it overruns.
    pub fn spawn(&self, command: &str, workspace: &Path, hard_limit: Duration) -> String {
        self.prune();

        let id = format!("job_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let kill = Arc::new(Notify::new());
        let state = Arc::new(Mutex::new(JobState {
            command: command.to_string(),
            status: JobStatus::Running,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            started_at: Instant::now(),
            finished_at: None,
            kill: kill.clone(),
        }));
        self.jobs.lock().insert(id.clone(), state.clone());

        let mut cmd = shell_command(command);
        cmd.current_dir(workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Intentionally NOT kill_on_drop: the whole point is to keep running.

        match cmd.spawn() {
            Ok(mut child) => {
                if let Some(out) = child.stdout.take() {
                    tokio::spawn(pump(out, state.clone(), false));
                }
                if let Some(err) = child.stderr.take() {
                    tokio::spawn(pump(err, state.clone(), true));
                }

                let st = state.clone();
                tokio::spawn(async move {
                    enum Outcome {
                        Exited(std::io::Result<std::process::ExitStatus>),
                        Stopped, // hard-timeout or explicit kill
                    }
                    let outcome = tokio::select! {
                        r = child.wait() => Outcome::Exited(r),
                        _ = tokio::time::sleep(hard_limit) => Outcome::Stopped,
                        _ = kill.notified() => Outcome::Stopped,
                    };
                    match outcome {
                        Outcome::Exited(Ok(status)) => {
                            let mut s = st.lock();
                            s.exit_code = status.code();
                            s.status = if status.success() {
                                JobStatus::Done
                            } else {
                                JobStatus::Failed
                            };
                            s.finished_at = Some(Instant::now());
                        }
                        Outcome::Exited(Err(_)) => {
                            let mut s = st.lock();
                            s.status = JobStatus::Failed;
                            s.finished_at = Some(Instant::now());
                        }
                        Outcome::Stopped => {
                            let _ = child.start_kill();
                            let _ = child.wait().await; // reap; no lock held
                            let mut s = st.lock();
                            s.status = JobStatus::Killed;
                            s.exit_code = Some(124);
                            s.finished_at = Some(Instant::now());
                        }
                    }
                });
            }
            Err(e) => {
                let mut s = state.lock();
                s.status = JobStatus::Failed;
                s.stderr = format!("Failed to spawn command: {e}");
                s.exit_code = Some(1);
                s.finished_at = Some(Instant::now());
            }
        }

        id
    }

    /// Current view of a job, or `None` if the id is unknown.
    pub fn snapshot(&self, job_id: &str) -> Option<JobSnapshot> {
        let state = self.jobs.lock().get(job_id).cloned()?;
        let s = state.lock();
        let end = s.finished_at.unwrap_or_else(Instant::now);
        Some(JobSnapshot {
            command: s.command.clone(),
            status: s.status,
            exit_code: s.exit_code,
            stdout: s.stdout.clone(),
            stderr: s.stderr.clone(),
            duration_ms: end.duration_since(s.started_at).as_millis() as u64,
            finished: s.status.is_terminal(),
        })
    }

    /// Request cancellation of a running job. Returns `true` if the job exists.
    pub fn kill(&self, job_id: &str) -> bool {
        if let Some(state) = self.jobs.lock().get(job_id).cloned() {
            state.lock().kill.notify_waiters();
            true
        } else {
            false
        }
    }

    /// Forget a job (used after its result has been consumed inline).
    pub fn remove(&self, job_id: &str) {
        self.jobs.lock().remove(job_id);
    }

    /// Drop the oldest finished jobs once the map grows too large.
    fn prune(&self) {
        let mut jobs = self.jobs.lock();
        if jobs.len() < MAX_RETAINED_JOBS {
            return;
        }
        let finished: Vec<String> = jobs
            .iter()
            .filter(|(_, st)| st.lock().status.is_terminal())
            .map(|(id, _)| id.clone())
            .collect();
        for id in finished {
            jobs.remove(&id);
        }
    }
}

/// Process-wide singleton.
pub fn manager() -> &'static BackgroundJobManager {
    static MANAGER: OnceLock<BackgroundJobManager> = OnceLock::new();
    MANAGER.get_or_init(BackgroundJobManager::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    async fn wait_terminal(mgr: &BackgroundJobManager, id: &str, max: Duration) -> JobSnapshot {
        let deadline = Instant::now() + max;
        loop {
            let snap = mgr.snapshot(id).expect("job exists");
            if snap.finished || Instant::now() >= deadline {
                return snap;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn captures_stdout_and_exit_code() {
        let mgr = BackgroundJobManager::new();
        let id = mgr.spawn("echo hello-bg", &ws(), Duration::from_secs(10));
        let snap = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;
        assert_eq!(snap.status, JobStatus::Done);
        assert_eq!(snap.exit_code, Some(0));
        assert!(
            snap.stdout.contains("hello-bg"),
            "stdout was: {:?}",
            snap.stdout
        );
    }

    #[tokio::test]
    async fn nonzero_exit_marks_failed() {
        let mgr = BackgroundJobManager::new();
        let id = mgr.spawn("exit 3", &ws(), Duration::from_secs(10));
        let snap = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;
        assert_eq!(snap.status, JobStatus::Failed);
        assert_eq!(snap.exit_code, Some(3));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn long_job_stays_running_then_can_be_killed() {
        let mgr = BackgroundJobManager::new();
        let id = mgr.spawn("sleep 30", &ws(), Duration::from_secs(60));
        // Still running shortly after spawn.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let running = mgr.snapshot(&id).unwrap();
        assert_eq!(running.status, JobStatus::Running);
        assert!(!running.finished);
        // Cancel it and confirm it reaches a terminal Killed state.
        assert!(mgr.kill(&id));
        let snap = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;
        assert_eq!(snap.status, JobStatus::Killed);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hard_limit_kills_overrun() {
        let mgr = BackgroundJobManager::new();
        let id = mgr.spawn("sleep 30", &ws(), Duration::from_millis(200));
        let snap = wait_terminal(&mgr, &id, Duration::from_secs(5)).await;
        assert_eq!(snap.status, JobStatus::Killed);
        assert_eq!(snap.exit_code, Some(124));
    }

    #[test]
    fn append_capped_keeps_tail_on_char_boundary() {
        let mut buf = String::new();
        let big = "é".repeat(MAX_JOB_OUTPUT_BYTES); // 2 bytes each → exceeds cap
        append_capped(&mut buf, &big);
        assert!(buf.len() <= MAX_JOB_OUTPUT_BYTES);
        // Still valid UTF-8 (no panic / no broken char) and non-empty.
        assert!(!buf.is_empty());
        assert!(buf.chars().all(|c| c == 'é'));
    }

    #[test]
    fn snapshot_none_for_unknown() {
        let mgr = BackgroundJobManager::new();
        assert!(mgr.snapshot("job_doesnotexist").is_none());
        assert!(!mgr.kill("job_doesnotexist"));
    }
}
