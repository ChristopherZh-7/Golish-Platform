//! PTY, shell, git, and terminal commands.

pub use crate::commands::proc::shell::{
    pty_create, pty_write, pty_resize, pty_destroy, pty_get_session,
    pty_get_foreground_process, set_active_terminal_session,
    list_path_completions, classify_input, shell_integration_status,
    shell_integration_install, shell_integration_uninstall,
};
pub use crate::commands::proc::git::{
    get_git_branch, git_status, git_diff, git_diff_staged,
    git_stage, git_unstage, git_commit, git_push, git_delete_worktree,
};
pub use crate::commands::proc::history::{
    add_command_history, add_prompt_history, load_history,
    search_history, clear_history,
};
pub use crate::commands::proc::command_index::run_recon_pipeline;
pub use crate::commands::proc::command_index::check_recon_tools_cmd;
pub use crate::commands::ui::themes::{
    list_themes, read_theme, save_theme, delete_theme,
    save_theme_asset, get_theme_asset_path,
};
pub use crate::commands::ui::logging::write_frontend_log;
pub use crate::commands::ui::ime::{ime_get_source, ime_set_source};
