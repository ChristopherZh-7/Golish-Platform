//! [`PtyManager::create_session_internal`] — generic session creation.
//!
//! Spawns the shell, wires up shell integration (ZDOTDIR / `--rcfile`),
//! resolves the working directory, opens a PTY pair, and starts the
//! reader/emitter thread pair.

use parking_lot::Mutex;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;

use uuid::Uuid;

use crate::error::{PtyError, Result};
use crate::parser::TerminalParser;
use crate::shell::{detect_shell, ShellIntegration};

use super::core::{ActiveSession, PtyManager, PtySession};
use super::emitter::PtyEventEmitter;
use super::utf8::OutputMessage;

mod emitter_loop;
mod reader;
mod util;

use self::emitter_loop::run_emitter_loop;
use self::reader::run_reader_loop;

impl PtyManager {
    /// Internal implementation that takes a generic emitter.
    ///
    /// Core session creation logic, abstracted over the event emission
    /// mechanism.
    pub(super) fn create_session_internal<E: PtyEventEmitter>(
        &self,
        emitter: Arc<E>,
        working_directory: Option<PathBuf>,
        rows: u16,
        cols: u16,
    ) -> Result<PtySession> {
        let session_id = Uuid::new_v4().to_string();

        tracing::info!(
            session_id = %session_id,
            rows = rows,
            cols = cols,
            requested_dir = ?working_directory,
            "Creating PTY session"
        );

        let pty_system = native_pty_system();

        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(size)
            .map_err(|e| PtyError::Pty(e.to_string()))?;

        // Detect shell from environment (settings integration can be
        // added later).
        let shell_env = std::env::var("SHELL").ok();
        let shell_info = detect_shell(None, shell_env.as_deref());

        tracing::info!(
            "Spawning shell: {} (detected type: {:?})",
            shell_info.path.display(),
            shell_info.shell_type()
        );

        let mut cmd = CommandBuilder::new(shell_info.path.to_str().unwrap_or("/bin/sh"));

        // Set up shell integration (ZDOTDIR for zsh, --rcfile for bash,
        // etc.). This injects OSC 133 sequences automatically without
        // requiring config-file edits.
        let integration = ShellIntegration::setup(shell_info.shell_type());

        // For shells with integration that provides custom args (like
        // bash --rcfile), use those instead of the default login args.
        let shell_args = integration.as_ref().map(|i| i.shell_args());
        if let Some(ref args) = shell_args {
            if !args.is_empty() {
                tracing::debug!(
                    session_id = %session_id,
                    args = ?args,
                    "Using integration shell args"
                );
                for arg in args {
                    cmd.arg(arg);
                }
            } else {
                cmd.args(shell_info.login_args());
            }
        } else {
            cmd.args(shell_info.login_args());
        }

        cmd.env("QBIT", "1");
        cmd.env("QBIT_VERSION", env!("CARGO_PKG_VERSION"));
        cmd.env("TERM", "xterm-256color");
        if std::env::var("LANG").is_err() {
            cmd.env("LANG", "en_US.UTF-8");
        }
        if std::env::var("LC_ALL").is_err() {
            cmd.env("LC_ALL", "en_US.UTF-8");
        }
        // Note: set QBIT_DEBUG=1 to enable shell integration debug output.

        // Set integration environment variables.
        if let Some(integration) = integration {
            for (key, value) in integration.env_vars() {
                tracing::debug!(
                    session_id = %session_id,
                    key = %key,
                    value = %value,
                    "Setting shell integration env var"
                );
                cmd.env(key, value);
            }
        }

        let (work_dir, dir_source) = if let Some(dir) = working_directory {
            (dir, "explicit")
        } else if let Ok(workspace) = std::env::var("QBIT_WORKSPACE") {
            // Expand ~ to home directory.
            let path = if let Some(stripped) = workspace.strip_prefix("~/") {
                if let Some(home) = dirs::home_dir() {
                    home.join(stripped)
                } else {
                    PathBuf::from(&workspace)
                }
            } else {
                PathBuf::from(&workspace)
            };
            (path, "QBIT_WORKSPACE")
        } else if let Ok(init_cwd) = std::env::var("INIT_CWD") {
            (PathBuf::from(init_cwd), "INIT_CWD")
        } else if let Ok(cwd) = std::env::current_dir() {
            // If cwd is root "/", fall through to home_dir — this
            // happens when launched from Finder.
            if cwd.as_os_str() == "/" {
                (
                    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
                    "home_dir (cwd was root)",
                )
            // If we're in src-tauri, go up to project root.
            } else if cwd.ends_with("src-tauri") {
                if let Some(parent) = cwd.parent() {
                    (parent.to_path_buf(), "current_dir (adjusted)")
                } else {
                    (cwd, "current_dir")
                }
            } else {
                (cwd, "current_dir")
            }
        } else {
            (
                dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
                "home_dir fallback",
            )
        };

        tracing::debug!(
            session_id = %session_id,
            work_dir = %work_dir.display(),
            source = dir_source,
            "Working directory resolved"
        );

        cmd.cwd(&work_dir);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Pty(e.to_string()))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Pty(e.to_string()))?;

        let master = Arc::new(Mutex::new(pair.master));

        // Erase the emitter's static type so it can be stored in
        // ActiveSession (which can't carry a generic parameter without
        // poisoning every call site). The reader thread + write-side
        // injection logic both work through this type-erased Arc.
        let emitter: Arc<dyn PtyEventEmitter> = emitter;

        // Shared parser so [`PtyManager::write`] can synthesize OSC
        // events (e.g. CommandStart for PowerShell on Windows) without
        // racing the reader thread's view of the parser state.
        let parser = Arc::new(Mutex::new(TerminalParser::new()));

        let alt_screen = Arc::new(AtomicBool::new(false));

        let session = Arc::new(ActiveSession {
            child: Mutex::new(child),
            master: master.clone(),
            writer: Mutex::new(writer),
            working_directory: Mutex::new(work_dir.clone()),
            rows: Mutex::new(rows),
            cols: Mutex::new(cols),
            shell_type: shell_info.shell_type(),
            parser: parser.clone(),
            emitter: emitter.clone(),
            alt_screen: alt_screen.clone(),
        });

        // Store session.
        {
            let mut sessions = self.sessions.lock();
            sessions.insert(session_id.clone(), session.clone());
        }

        // Start read thread with the generic emitter.
        let reader_session_id = session_id.clone();
        let reader_session = session.clone();
        let reader_emitter = emitter.clone();
        let reader_parser = parser.clone();
        let reader_grid_manager = self.grid_manager.clone();

        // Get a reader from the master.
        let reader = {
            let master = master.lock();
            master
                .try_clone_reader()
                .map_err(|e| PtyError::Pty(e.to_string()))?
        };

        // Channel for passing raw output bytes from the reader thread to
        // the emitter thread. Allows the emitter to coalesce bursts of
        // small reads into batched IPC events (~60 fps / 16 ms window).
        let (output_tx, output_rx) = std::sync::mpsc::channel::<OutputMessage>();

        // Clone emitter for the output emitter thread (reader keeps the
        // original).
        let emitter_for_output = emitter.clone();
        let output_session_id = session_id.clone();
        let emitter_grid_manager = self.grid_manager.clone();
        let emitter_alt_screen = alt_screen.clone();

        // Spawn reader thread.
        let reader_session_id_for_log = reader_session_id.clone();
        tracing::trace!(
            session_id = %reader_session_id_for_log,
            "Spawning PTY reader thread"
        );

        thread::spawn(move || {
            run_reader_loop(
                reader,
                reader_session_id,
                reader_session,
                reader_emitter,
                reader_parser,
                reader_grid_manager,
                output_tx,
            )
        });

        // Spawn output emitter thread (see `emitter_loop::run_emitter_loop`).
        thread::spawn(move || {
            run_emitter_loop(
                output_rx,
                emitter_for_output,
                output_session_id,
                emitter_grid_manager,
                emitter_alt_screen,
            )
        });

        Ok(PtySession {
            id: session_id,
            working_directory: work_dir.to_string_lossy().to_string(),
            rows,
            cols,
        })
    }
}
