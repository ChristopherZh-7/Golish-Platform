//! PTY, shell, git, terminal, themes, IME, and frontend-log commands.
//!
//! Expected command domains exposed here (documentation only):
//! - **PTY**: `pty_create`, `pty_write`, `pty_resize`, `pty_destroy`,
//!   `pty_get_session`, `pty_get_foreground_process`,
//!   `set_active_terminal_session`
//! - **Path completion & shell**: `list_path_completions`,
//!   `classify_input`, `shell_integration_{status,install,uninstall}`
//! - **Git**: `get_git_branch`, `git_{status,diff,diff_staged,stage,
//!   unstage,commit,push,delete_worktree}`
//! - **History**: `add_command_history`, `add_prompt_history`,
//!   `load_history`, `search_history`, `clear_history`
//! - **Recon pipeline**: `run_recon_pipeline`, `check_recon_tools_cmd`
//! - **Themes**: `list_themes`, `read_theme`, `save_theme`,
//!   `delete_theme`, `save_theme_asset`, `get_theme_asset_path`
//! - **Frontend log**: `write_frontend_log`
//! - **IME (macOS)**: `ime_get_source`, `ime_set_source`

pub use crate::commands::proc::*;
pub use crate::commands::ui::*;
