//! Cross-platform abstraction layer for the Golish platform workspace.
//!
//! This crate is the **single place** where conditional-compilation
//! tricks (`#[cfg(target_os = "...")]`, `cfg!(windows)`, hard-coded
//! `Command::new("sh")`, hard-coded shared-library extensions, etc.)
//! are allowed to live. Every other crate must depend on it instead of
//! sprinkling its own platform branches.
//!
//! ## Design rules
//!
//! - **No `#[cfg(target_os = …)]` outside this crate.** Application
//!   code calls a method on [`Platform`] and gets the right answer for
//!   the current OS.
//! - **Capabilities, not platform names.** Methods are named after
//!   "what the caller wants" (e.g. [`Platform::default_shell`]) not
//!   "what platform we're on".
//! - **Trivially mockable.** `Platform::current()` is a thin facade
//!   over module-level free functions; tests can target the modules
//!   directly.
//!
//! ## Module layout
//!
//! - [`detect`]   — [`PlatformKind`] / [`Arch`] enum + detection
//! - [`shell`]    — [`shell::default_shell_invocation`],
//!   [`shell::build_shell_command`], [`shell::which_executable`]
//! - [`process`]  — [`process::kill_pid`],
//!   [`process::pids_listening_on_port`], process-group helpers
//! - [`paths`]    — extension constants (`EXE`, `DYLIB`), `dirs::*` wrappers
//! - [`fs_perms`] — [`fs_perms::set_executable`], [`fs_perms::has_execute_bit`]
//! - [`open`]     — [`open::open_url`], [`open::reveal_path`]
//! - [`package_manager`] — package-manager install hints and
//!   [`package_manager::PackageManager`]
//! - [`postgres`] — embedded PostgreSQL / pgvector platform helpers
//! - [`system_proxy`] — desktop system proxy control helpers

pub mod detect;
pub mod fs_perms;
pub mod open;
pub mod package_manager;
pub mod paths;
pub mod postgres;
pub mod process;
pub mod shell;
pub mod system_proxy;

pub use detect::{Arch, Platform, PlatformKind};
pub use package_manager::PackageManager;
