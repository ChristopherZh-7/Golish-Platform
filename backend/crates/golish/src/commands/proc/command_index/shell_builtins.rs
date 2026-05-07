//! Shell-type detection and built-in command tables.
//!
//! These tables exist to compensate for the fact that shell builtins are
//! never installed as standalone executables on `PATH`, so a pure PATH-based
//! index would mis-classify e.g. `cd foo` as natural language.
//!
//! Tables are intentionally static and per-shell — keeping them here means
//! the orchestration code in `mod.rs` doesn't need to scroll past 200+ lines
//! of string literals to read the `CommandIndex` logic.

use golish_pty::ShellType;

/// Detect the shell type from the `SHELL` environment variable.
pub(super) fn detect_shell_type() -> ShellType {
    let shell = std::env::var("SHELL").unwrap_or_default();
    if shell.ends_with("/zsh") || shell.ends_with("/zsh5") {
        ShellType::Zsh
    } else if shell.ends_with("/bash") {
        ShellType::Bash
    } else if shell.ends_with("/fish") {
        ShellType::Fish
    } else {
        ShellType::Unknown
    }
}

/// Return shell builtins for the given shell type.
pub(super) fn shell_builtins(shell_type: ShellType) -> &'static [&'static str] {
    match shell_type {
        ShellType::Zsh => ZSH_BUILTINS,
        ShellType::Bash => BASH_BUILTINS,
        ShellType::Fish => FISH_BUILTINS,
        ShellType::Sh => UNKNOWN_BUILTINS,
        ShellType::PowerShell => POWERSHELL_BUILTINS,
        ShellType::Cmd => CMD_BUILTINS,
        ShellType::Unknown => UNKNOWN_BUILTINS,
    }
}

const ZSH_BUILTINS: &[&str] = &[
    "alias",
    "autoload",
    "bg",
    "bindkey",
    "builtin",
    "cd",
    "command",
    "compdef",
    "compadd",
    "declare",
    "dirs",
    "disown",
    "echo",
    "emulate",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "float",
    "functions",
    "getln",
    "hash",
    "history",
    "integer",
    "jobs",
    "kill",
    "let",
    "limit",
    "local",
    "log",
    "logout",
    "noglob",
    "popd",
    "print",
    "printf",
    "pushd",
    "pushln",
    "pwd",
    "read",
    "readonly",
    "rehash",
    "return",
    "sched",
    "set",
    "setopt",
    "shift",
    "source",
    "stat",
    "suspend",
    "test",
    "times",
    "trap",
    "true",
    "ttyctl",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unfunction",
    "unhash",
    "unlimit",
    "unset",
    "unsetopt",
    "vared",
    "wait",
    "whence",
    "where",
    "which",
    "zcompile",
    "zformat",
    "zle",
    "zmodload",
    "zparseopts",
    "zstyle",
];

const BASH_BUILTINS: &[&str] = &[
    "alias",
    "bg",
    "bind",
    "break",
    "builtin",
    "caller",
    "cd",
    "command",
    "compgen",
    "complete",
    "compopt",
    "continue",
    "declare",
    "dirs",
    "disown",
    "echo",
    "enable",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "getopts",
    "hash",
    "help",
    "history",
    "jobs",
    "kill",
    "let",
    "local",
    "logout",
    "mapfile",
    "popd",
    "printf",
    "pushd",
    "pwd",
    "read",
    "readarray",
    "readonly",
    "return",
    "set",
    "shift",
    "shopt",
    "source",
    "suspend",
    "test",
    "times",
    "trap",
    "true",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unset",
    "wait",
];

const FISH_BUILTINS: &[&str] = &[
    "abbr",
    "alias",
    "and",
    "argparse",
    "begin",
    "bg",
    "bind",
    "block",
    "break",
    "breakpoint",
    "builtin",
    "case",
    "cd",
    "command",
    "commandline",
    "complete",
    "contains",
    "continue",
    "count",
    "disown",
    "echo",
    "else",
    "emit",
    "end",
    "eval",
    "exec",
    "exit",
    "false",
    "fg",
    "for",
    "function",
    "functions",
    "history",
    "if",
    "jobs",
    "math",
    "not",
    "or",
    "popd",
    "printf",
    "pushd",
    "pwd",
    "random",
    "read",
    "realpath",
    "return",
    "set",
    "set_color",
    "source",
    "status",
    "string",
    "suspend",
    "switch",
    "test",
    "time",
    "trap",
    "true",
    "type",
    "ulimit",
    "wait",
    "while",
];

const POWERSHELL_BUILTINS: &[&str] = &[
    "Add-Content",
    "Clear-Content",
    "Clear-Host",
    "Clear-Item",
    "Copy-Item",
    "ForEach-Object",
    "Get-ChildItem",
    "Get-Command",
    "Get-Content",
    "Get-Help",
    "Get-Item",
    "Get-Location",
    "Get-Process",
    "Get-Service",
    "Invoke-Expression",
    "Invoke-WebRequest",
    "Move-Item",
    "New-Item",
    "Out-File",
    "Remove-Item",
    "Rename-Item",
    "Select-Object",
    "Set-Content",
    "Set-Item",
    "Set-Location",
    "Sort-Object",
    "Start-Process",
    "Stop-Process",
    "Test-Path",
    "Where-Object",
    "Write-Host",
    "Write-Output",
    "cd",
    "cls",
    "copy",
    "del",
    "dir",
    "echo",
    "exit",
    "ls",
    "mkdir",
    "mv",
    "pwd",
    "rm",
    "type",
];

const CMD_BUILTINS: &[&str] = &[
    "assoc", "attrib", "break", "call", "cd", "chdir", "cls", "color", "copy", "date", "del",
    "dir", "echo", "endlocal", "erase", "exit", "for", "ftype", "goto", "if", "md", "mkdir",
    "mklink", "move", "path", "pause", "popd", "prompt", "pushd", "rd", "rem", "ren", "rename",
    "rmdir", "set", "setlocal", "shift", "start", "time", "title", "tree", "type", "ver", "vol",
];

const UNKNOWN_BUILTINS: &[&str] = &[
    "alias", "bg", "cd", "command", "echo", "eval", "exec", "exit", "export", "false", "fc", "fg",
    "getopts", "hash", "jobs", "kill", "local", "popd", "printf", "pushd", "pwd", "read",
    "readonly", "return", "set", "shift", "source", "test", "times", "trap", "true", "type",
    "ulimit", "umask", "unalias", "unset", "wait",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zsh_builtins_contain_essential_commands() {
        let zsh = shell_builtins(ShellType::Zsh);
        assert!(zsh.contains(&"cd"));
        assert!(zsh.contains(&"setopt"));
        assert!(zsh.contains(&"zle"));
    }

    #[test]
    fn bash_builtins_contain_essential_commands() {
        let bash = shell_builtins(ShellType::Bash);
        assert!(bash.contains(&"cd"));
        assert!(bash.contains(&"shopt"));
        assert!(bash.contains(&"compgen"));
    }

    #[test]
    fn unknown_falls_back_to_posix_subset() {
        let unk = shell_builtins(ShellType::Unknown);
        assert!(unk.contains(&"cd"));
        assert!(unk.contains(&"export"));
    }
}
