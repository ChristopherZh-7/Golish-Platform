use std::path::{Path, PathBuf};

use serde_json::json;
use tempfile::tempdir;

use crate::common::truncate_output;
use crate::shell::{get_shell_config, ShellType};
use crate::tool::RunPtyCmdTool;
use golish_core::Tool;

// ────────── Shell detection tests ──────────────────────────────────────

#[test]
fn test_shell_type_from_path_zsh() {
    assert_eq!(ShellType::from_path(Path::new("/bin/zsh")), ShellType::Zsh);
    assert_eq!(
        ShellType::from_path(Path::new("/usr/local/bin/zsh")),
        ShellType::Zsh
    );
    assert_eq!(
        ShellType::from_path(Path::new("/opt/homebrew/bin/zsh")),
        ShellType::Zsh
    );
}

#[test]
fn test_shell_type_from_path_bash() {
    assert_eq!(
        ShellType::from_path(Path::new("/bin/bash")),
        ShellType::Bash
    );
    assert_eq!(
        ShellType::from_path(Path::new("/usr/bin/bash")),
        ShellType::Bash
    );
}

#[test]
fn test_shell_type_from_path_fish() {
    assert_eq!(
        ShellType::from_path(Path::new("/usr/bin/fish")),
        ShellType::Fish
    );
    assert_eq!(
        ShellType::from_path(Path::new("/opt/homebrew/bin/fish")),
        ShellType::Fish
    );
}

#[test]
fn test_shell_type_from_path_sh() {
    assert_eq!(ShellType::from_path(Path::new("/bin/sh")), ShellType::Sh);
    assert_eq!(ShellType::from_path(Path::new("/bin/dash")), ShellType::Sh);
    assert_eq!(ShellType::from_path(Path::new("/bin/tcsh")), ShellType::Sh);
}

#[test]
fn test_shell_type_rc_file_zsh() {
    let home = PathBuf::from("/home/user");
    assert_eq!(
        ShellType::Zsh.rc_file(&home),
        Some(PathBuf::from("/home/user/.zshrc"))
    );
}

#[test]
fn test_shell_type_rc_file_fish() {
    let home = PathBuf::from("/home/user");
    assert_eq!(
        ShellType::Fish.rc_file(&home),
        Some(PathBuf::from("/home/user/.config/fish/config.fish"))
    );
}

#[test]
fn test_shell_type_rc_file_sh() {
    let home = PathBuf::from("/home/user");
    assert_eq!(ShellType::Sh.rc_file(&home), None);
}

#[test]
fn test_build_command_zsh_with_rc() {
    let dir = tempdir().unwrap();
    let home = dir.path();
    std::fs::write(home.join(".zshrc"), "# zshrc").unwrap();

    let (shell, cmd) = ShellType::Zsh.build_command(Path::new("/bin/zsh"), "echo hello", home);

    assert_eq!(shell, "/bin/zsh");
    assert!(cmd.contains("source"));
    assert!(cmd.contains(".zshrc"));
    assert!(cmd.contains("echo hello"));
}

#[test]
fn test_build_command_zsh_without_rc() {
    let dir = tempdir().unwrap();
    let home = dir.path();

    let (shell, cmd) = ShellType::Zsh.build_command(Path::new("/bin/zsh"), "echo hello", home);

    assert_eq!(shell, "/bin/zsh");
    assert_eq!(cmd, "echo hello");
}

#[test]
fn test_build_command_bash_with_bashrc() {
    let dir = tempdir().unwrap();
    let home = dir.path();
    std::fs::write(home.join(".bashrc"), "# bashrc").unwrap();

    let (shell, cmd) = ShellType::Bash.build_command(Path::new("/bin/bash"), "echo hello", home);

    assert_eq!(shell, "/bin/bash");
    assert!(cmd.contains("source"));
    assert!(cmd.contains(".bashrc"));
    assert!(cmd.contains("echo hello"));
}

#[test]
fn test_build_command_bash_with_bash_profile() {
    let dir = tempdir().unwrap();
    let home = dir.path();
    std::fs::write(home.join(".bash_profile"), "# bash_profile").unwrap();

    let (shell, cmd) = ShellType::Bash.build_command(Path::new("/bin/bash"), "echo hello", home);

    assert_eq!(shell, "/bin/bash");
    assert!(cmd.contains("source"));
    assert!(cmd.contains(".bash_profile"));
    assert!(cmd.contains("echo hello"));
}

#[test]
fn test_build_command_sh() {
    let dir = tempdir().unwrap();
    let home = dir.path();

    let (shell, cmd) = ShellType::Sh.build_command(Path::new("/bin/sh"), "echo hello", home);

    assert_eq!(shell, "/bin/sh");
    assert_eq!(cmd, "echo hello");
}

#[test]
fn test_build_command_fish_with_config() {
    let dir = tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir_all(home.join(".config/fish")).unwrap();
    std::fs::write(home.join(".config/fish/config.fish"), "# fish config").unwrap();

    let (shell, cmd) = ShellType::Fish.build_command(Path::new("/usr/bin/fish"), "echo hello", home);

    assert_eq!(shell, "/usr/bin/fish");
    assert!(cmd.contains("source"));
    assert!(cmd.contains("config.fish"));
    assert!(cmd.contains("echo hello"));
}

// ────────── Integration tests ─────────────────────────────────────────

#[tokio::test]
async fn test_run_pty_cmd_echo() {
    let dir = tempdir().unwrap();

    let tool = RunPtyCmdTool::new();
    let result = tool
        .execute(json!({"command": "echo hello"}), dir.path())
        .await
        .unwrap();

    assert_eq!(result["exit_code"].as_i64(), Some(0));
    assert!(result.get("error").is_none());
    assert!(result["stdout"].as_str().unwrap().contains("hello"));
}

#[tokio::test]
async fn test_run_pty_cmd_exit_code() {
    let dir = tempdir().unwrap();

    let tool = RunPtyCmdTool::new();
    let result = tool
        .execute(json!({"command": "exit 42"}), dir.path())
        .await
        .unwrap();

    assert_eq!(result["exit_code"].as_i64(), Some(42));
    assert!(result.get("error").is_some());
}

#[tokio::test]
async fn test_run_pty_cmd_with_cwd() {
    let dir = tempdir().unwrap();
    let workspace = dir.path();

    std::fs::create_dir(workspace.join("subdir")).unwrap();

    let tool = RunPtyCmdTool::new();
    let result = tool
        .execute(json!({"command": "pwd", "cwd": "subdir"}), workspace)
        .await
        .unwrap();

    assert_eq!(result["exit_code"].as_i64(), Some(0));
    assert!(result["stdout"].as_str().unwrap().contains("subdir"));
}

#[tokio::test]
async fn test_run_pty_cmd_stderr() {
    let dir = tempdir().unwrap();

    let tool = RunPtyCmdTool::new();
    let result = tool
        .execute(json!({"command": "echo error >&2"}), dir.path())
        .await
        .unwrap();

    assert_eq!(result["exit_code"].as_i64(), Some(0));
    assert!(result["stderr"].as_str().unwrap().contains("error"));
}

#[tokio::test]
async fn test_run_pty_cmd_timeout() {
    let dir = tempdir().unwrap();

    let tool = RunPtyCmdTool::new();
    let result = tool
        .execute(json!({"command": "sleep 10", "timeout": 1}), dir.path())
        .await
        .unwrap();

    assert!(result.get("error").is_some());
    assert!(result["error"].as_str().unwrap().contains("timed out"));
    assert_eq!(result["exit_code"].as_i64(), Some(124));
}

#[tokio::test]
async fn test_run_pty_cmd_missing_command() {
    let dir = tempdir().unwrap();

    let tool = RunPtyCmdTool::new();
    let result = tool.execute(json!({}), dir.path()).await.unwrap();

    assert!(result.get("error").is_some());
    assert!(result["error"].as_str().unwrap().contains("Missing"));
}

#[tokio::test]
async fn test_run_pty_cmd_invalid_cwd() {
    let dir = tempdir().unwrap();

    let tool = RunPtyCmdTool::new();
    let result = tool
        .execute(
            json!({"command": "echo test", "cwd": "nonexistent"}),
            dir.path(),
        )
        .await
        .unwrap();

    assert!(result.get("error").is_some());
    assert!(result["error"].as_str().unwrap().contains("not found"));
}

#[tokio::test]
async fn test_run_pty_cmd_pipe() {
    let dir = tempdir().unwrap();

    let tool = RunPtyCmdTool::new();
    let result = tool
        .execute(
            json!({"command": "echo 'hello world' | grep hello"}),
            dir.path(),
        )
        .await
        .unwrap();

    assert_eq!(result["exit_code"].as_i64(), Some(0));
    assert!(result["stdout"].as_str().unwrap().contains("hello"));
}

#[tokio::test]
async fn test_run_pty_cmd_multiline() {
    let dir = tempdir().unwrap();

    let tool = RunPtyCmdTool::new();
    let result = tool
        .execute(json!({"command": "echo line1 && echo line2"}), dir.path())
        .await
        .unwrap();

    assert_eq!(result["exit_code"].as_i64(), Some(0));
    let stdout = result["stdout"].as_str().unwrap();
    assert!(stdout.contains("line1"));
    assert!(stdout.contains("line2"));
}

#[test]
fn test_truncate_output_short() {
    let content = b"short content";
    let result = truncate_output(content, 1000);
    assert_eq!(result, "short content");
}

#[test]
fn test_truncate_output_long() {
    let content = b"a".repeat(1000);
    let result = truncate_output(&content, 100);
    assert!(result.contains("[Output truncated"));
    assert!(result.len() < 200); // some overhead for the message
}

// ────────── Shell override tests ──────────────────────────────────────

#[test]
fn test_get_shell_config_with_override() {
    let (shell_path, shell_type, _home) = get_shell_config(Some("/usr/local/bin/fish"));
    assert_eq!(shell_path.to_string_lossy(), "/usr/local/bin/fish");
    assert_eq!(shell_type, ShellType::Fish);
}

#[test]
fn test_get_shell_config_with_zsh_override() {
    let (shell_path, shell_type, _home) = get_shell_config(Some("/opt/homebrew/bin/zsh"));
    assert_eq!(shell_path.to_string_lossy(), "/opt/homebrew/bin/zsh");
    assert_eq!(shell_type, ShellType::Zsh);
}

#[test]
fn test_get_shell_config_with_bash_override() {
    let (shell_path, shell_type, _home) = get_shell_config(Some("/bin/bash"));
    assert_eq!(shell_path.to_string_lossy(), "/bin/bash");
    assert_eq!(shell_type, ShellType::Bash);
}

#[test]
fn test_get_shell_config_none_falls_back_to_env_or_default() {
    // When shell_override is None, it should try $SHELL then fall back to /bin/sh.
    let (shell_path, _shell_type, _home) = get_shell_config(None);
    assert!(!shell_path.to_string_lossy().is_empty());
}

#[test]
fn test_run_pty_cmd_tool_with_shell_override() {
    let tool = RunPtyCmdTool::with_shell(Some("/bin/zsh".to_string()));
    assert_eq!(tool.shell_override, Some("/bin/zsh".to_string()));
}

#[test]
fn test_run_pty_cmd_tool_default() {
    let tool = RunPtyCmdTool::new();
    assert_eq!(tool.shell_override, None);
}

#[tokio::test]
async fn test_run_pty_cmd_with_shell_override() {
    let dir = tempdir().unwrap();

    // Use /bin/sh as override (should be available on all Unix systems).
    let tool = RunPtyCmdTool::with_shell(Some("/bin/sh".to_string()));
    let result = tool
        .execute(json!({"command": "echo shell_override_test"}), dir.path())
        .await
        .unwrap();

    assert_eq!(result["exit_code"].as_i64(), Some(0));
    assert!(result["stdout"]
        .as_str()
        .unwrap()
        .contains("shell_override_test"));
}
