//! Cross-platform helpers for "fire-and-forget" shell command execution
//! and executable lookup.
//!
//! The actual implementation now lives in [`golish_platform::shell`].
//! This module is preserved purely as a thin re-export so existing
//! callers compile unchanged. New code should import from
//! `golish_platform::shell` directly.

pub use golish_platform::shell::{
    build_shell_command, build_tokio_shell_command, default_shell_invocation, lookup_program,
    which_executable, which_executable_async,
};
