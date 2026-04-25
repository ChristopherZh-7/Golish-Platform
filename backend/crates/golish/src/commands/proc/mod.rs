//! Process / shell / terminal-domain Tauri commands.
//!
//! - [`shell`]: cross-platform shell execution and integration installer
//! - [`pty`]: PTY session lifecycle wired to `golish-pty`
//! - [`command_index`]: PATH index used to classify input as command vs prompt
//! - [`git`]: porcelain commands invoked from the editor commit panel
//! - [`history`]: shell + prompt history management
//!
//! All public items are re-exported at the parent [`crate::commands`] level.

mod command_index;
mod git;
mod history;
mod pty;
mod shell;

pub use command_index::*;
pub use git::*;
pub use history::*;
pub use pty::*;
pub use shell::*;
