//! Platform-specific process-group handling for spawned shell commands.
//!
//! On Unix we put each spawned shell in its own process group so we can
//! reliably kill the entire pipeline (e.g. `cmd1 | cmd2 | cmd3`) on
//! timeout or cancellation. On non-Unix platforms these helpers degrade to
//! `child.kill()` only.

use tokio::process::Command;

#[cfg(unix)]
use nix::sys::signal::{killpg, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

#[cfg(unix)]
pub(crate) fn configure_process_group(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            // Create a new process group for the spawned shell.
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
pub(crate) fn configure_process_group(_cmd: &mut Command) {}

#[cfg(unix)]
pub(crate) async fn kill_process_group(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL);
    }
    let _ = child.kill().await;
}

#[cfg(not(unix))]
pub(crate) async fn kill_process_group(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
}
