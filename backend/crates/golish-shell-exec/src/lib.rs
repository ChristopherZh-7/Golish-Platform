//! Shell command execution.
//!
//! Provides shell command execution with proper PATH inheritance from the
//! user's shell configuration files (`.zshrc`, `.bashrc`, etc.) plus a
//! streaming variant for long-running commands.
//!
//! ## Layout
//!
//! - [`process_group`]: Unix process-group helpers used to clean up
//!   pipelines on timeout / cancel.
//! - [`shell`]: shell-type detection (`zsh` / `bash` / `fish` / `sh`) and
//!   rc-file-aware command wrapping.
//! - [`common`]: shared constants ([`common::DEFAULT_TIMEOUT_SECS`],
//!   [`common::MAX_OUTPUT_SIZE`]) and small helpers
//!   ([`common::resolve_cwd`], [`common::truncate_output`]).
//! - [`streaming`]: [`streaming::execute_streaming`] for real-time output
//!   chunks via mpsc channel.
//! - [`tool`]: [`tool::RunPtyCmdTool`] — synchronous tool registered with
//!   the agent's tool registry.
//!
//! ## Streaming Support
//!
//! For long-running commands, use
//! [`streaming::execute_streaming`] instead of
//! [`tool::RunPtyCmdTool::execute`] to receive output chunks as they
//! arrive. This provides real-time feedback without waiting for the
//! command to complete.

mod common;
mod process_group;
mod shell;
mod streaming;
mod tool;

#[cfg(test)]
mod tests;

// Re-export the Tool trait from golish-core for convenience.
pub use golish_core::Tool;

pub use streaming::{execute_streaming, OutputChunk, OutputStream, StreamingResult};
pub use tool::RunPtyCmdTool;
