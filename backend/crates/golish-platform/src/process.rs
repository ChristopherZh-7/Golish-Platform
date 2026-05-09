//! Cross-platform process / signal management.
//!
//! Wraps Unix `libc::kill` / `pkill` / `lsof` and Windows `taskkill` /
//! `tasklist` / `netstat` behind one interface. Callers never write
//! `cfg!(windows)` blocks; they call [`kill_pid`] or
//! [`pids_listening_on_port`] and the right thing happens.

use crate::detect::Platform;

/// Send a "please terminate" request to the given PID.
///
/// `force = true` corresponds to SIGKILL on Unix and `taskkill /F /T`
/// on Windows. `force = false` corresponds to SIGTERM / `taskkill /T`.
pub fn kill_pid(pid: u32, force: bool) -> bool {
    #[cfg(target_os = "windows")]
    {
        windows::taskkill(pid, force)
    }
    #[cfg(unix)]
    {
        unix::kill(pid, force)
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = (pid, force);
        false
    }
}

/// Return true if a process with `pid` is alive.
pub fn is_pid_running(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        windows::pid_alive(pid)
    }
    #[cfg(unix)]
    {
        unix::pid_alive(pid)
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = pid;
        false
    }
}

/// Find PIDs of processes currently listening on TCP `port`.
pub fn pids_listening_on_port(port: u16) -> Vec<u32> {
    if Platform::current().is_windows() {
        #[cfg(target_os = "windows")]
        {
            return windows::pids_on_port(port);
        }
    } else {
        #[cfg(unix)]
        {
            return unix::pids_on_port(port);
        }
    }
    Vec::new()
}

/// Force-terminate any process currently listening on `port`.
///
/// `force = true` immediately escalates to SIGKILL / `/F`. Returns the
/// number of PIDs targeted (whether the kill succeeded or not — the
/// caller should re-probe the port).
pub fn kill_processes_on_port(port: u16, force: bool) -> usize {
    let pids = pids_listening_on_port(port);
    let count = pids.len();
    for pid in pids {
        let _ = kill_pid(pid, force);
    }
    count
}

/// Return the best-effort foreground process name for the current terminal.
///
/// Unix platforms expose terminal foreground process groups in a way we can
/// query through `ps`; Windows does not have the same semantics here, so it
/// returns `None`.
pub fn foreground_process_name() -> Option<String> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let output = crate::shell::build_shell_command(
            "ps -o comm= -p $(ps -o tpgid= -p $$) 2>/dev/null || echo ''",
        )
        .output()
        .ok()?;

        if !output.status.success() {
            return None;
        }

        normalize_process_name(&String::from_utf8_lossy(&output.stdout))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn normalize_process_name(output: &str) -> Option<String> {
    let process_name = output.trim();
    if process_name.is_empty() {
        return None;
    }

    Some(
        process_name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(process_name)
            .to_string(),
    )
}

#[cfg(unix)]
mod unix {
    pub fn kill(pid: u32, force: bool) -> bool {
        let signum = if force { libc::SIGKILL } else { libc::SIGTERM };
        unsafe { libc::kill(pid as i32, signum) == 0 }
    }

    pub fn pid_alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }

    pub fn pids_on_port(port: u16) -> Vec<u32> {
        let output = match std::process::Command::new("lsof")
            .args(["-ti", &format!(":{}", port)])
            .output()
        {
            Ok(o) if o.status.success() => o,
            Ok(_) | Err(_) => return Vec::new(),
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .split_whitespace()
            .filter_map(|s| s.parse::<u32>().ok())
            .collect()
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use tracing::{debug, warn};

    pub fn taskkill(pid: u32, force: bool) -> bool {
        let pid_s = pid.to_string();
        let mut args: Vec<&str> = vec!["/PID", &pid_s, "/T"];
        if force {
            args.push("/F");
        }
        match std::process::Command::new("taskkill").args(&args).output() {
            Ok(out) if out.status.success() => true,
            Ok(out) => {
                warn!(
                    "taskkill /PID {} returned exit {:?}: {}",
                    pid,
                    out.status.code(),
                    String::from_utf8_lossy(&out.stderr).trim()
                );
                false
            }
            Err(e) => {
                warn!("Failed to spawn taskkill for PID {}: {}", pid, e);
                false
            }
        }
    }

    pub fn pid_alive(pid: u32) -> bool {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| {
                let s = String::from_utf8_lossy(&o.stdout);
                s.contains(&pid.to_string())
            })
            .unwrap_or(false)
    }

    pub fn pids_on_port(port: u16) -> Vec<u32> {
        let output = match std::process::Command::new("netstat")
            .args(["-ano"])
            .output()
        {
            Ok(o) if o.status.success() => o,
            Ok(_) | Err(_) => return Vec::new(),
        };
        let stdout = String::from_utf8_lossy(&output.stdout);
        let needle_v4 = format!(":{} ", port);
        let needle_v6 = format!("]:{}", port);
        let mut pids = Vec::new();
        for line in stdout.lines() {
            let trimmed = line.trim();
            if !trimmed.to_uppercase().contains("LISTENING") {
                continue;
            }
            if !trimmed.contains(&needle_v4) && !trimmed.contains(&needle_v6) {
                continue;
            }
            if let Some(pid_str) = trimmed.split_whitespace().last() {
                if let Ok(pid) = pid_str.parse::<u32>() {
                    if !pids.contains(&pid) {
                        pids.push(pid);
                    }
                }
            }
        }
        debug!(
            "Found {} PIDs listening on port {}: {:?}",
            pids.len(),
            port,
            pids
        );
        pids
    }
}

/// Configure a freshly-built [`tokio::process::Command`] so that the
/// spawned child runs in its own process group on Unix. On Windows
/// this is a no-op (Windows doesn't have process groups in the Unix
/// sense; see `kill_pid` for tree-kill behaviour).
pub fn configure_process_group(cmd: &mut tokio::process::Command) {
    #[cfg(unix)]
    {
        unsafe {
            cmd.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    {
        let _ = cmd;
    }
}

/// Kill a tokio child plus its entire process group on Unix; on
/// Windows just kills the child (and `kill_pid` should be used for
/// tree-kill via `taskkill /T`).
pub async fn kill_process_group(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
    let _ = child.kill().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_name_strips_path_and_empty_output() {
        assert_eq!(
            normalize_process_name("/usr/local/bin/npm\n"),
            Some("npm".to_string())
        );
        assert_eq!(normalize_process_name("cargo"), Some("cargo".to_string()));
        assert_eq!(normalize_process_name("\n"), None);
    }
}
