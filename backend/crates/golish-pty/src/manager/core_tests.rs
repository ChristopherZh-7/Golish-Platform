use super::*;

#[test]
fn synthetic_command_start_only_for_powershell() {
    assert!(PtyManager::needs_synthetic_command_start(
        ShellType::PowerShell
    ));
    assert!(!PtyManager::needs_synthetic_command_start(ShellType::Zsh));
    assert!(!PtyManager::needs_synthetic_command_start(ShellType::Bash));
    assert!(!PtyManager::needs_synthetic_command_start(ShellType::Fish));
    assert!(!PtyManager::needs_synthetic_command_start(ShellType::Sh));
    assert!(!PtyManager::needs_synthetic_command_start(ShellType::Cmd));
    assert!(!PtyManager::needs_synthetic_command_start(
        ShellType::Unknown
    ));
}

#[test]
fn extracts_simple_command() {
    assert_eq!(
        PtyManager::extract_injected_command(b"ls\n"),
        Some("ls".to_string())
    );
}

#[test]
fn extracts_command_with_args() {
    assert_eq!(
        PtyManager::extract_injected_command(b"git status --short\n"),
        Some("git status --short".to_string())
    );
}

#[test]
fn extracts_command_with_crlf() {
    assert_eq!(
        PtyManager::extract_injected_command(b"pwd\r\n"),
        Some("pwd".to_string())
    );
}

#[test]
fn trims_leading_and_trailing_whitespace() {
    assert_eq!(
        PtyManager::extract_injected_command(b"  echo hi  \n"),
        Some("echo hi".to_string())
    );
}

#[test]
fn rejects_writes_without_newline() {
    // Partial typing should not synthesize a CommandStart — wait for
    // the user (or the input box) to actually submit the command.
    assert_eq!(PtyManager::extract_injected_command(b"ls"), None);
}

#[test]
fn rejects_blank_line() {
    assert_eq!(PtyManager::extract_injected_command(b"\n"), None);
    assert_eq!(PtyManager::extract_injected_command(b"   \n"), None);
}

#[test]
fn rejects_control_only_payload() {
    // Ctrl-C (0x03) and similar control bytes are written by the
    // shortcut handlers, never accompanied by a real command.
    assert_eq!(PtyManager::extract_injected_command(b"\x03\n"), None);
    assert_eq!(PtyManager::extract_injected_command(b"\x04\n"), None);
}

#[test]
fn rejects_non_utf8_payload() {
    assert_eq!(
        PtyManager::extract_injected_command(&[0xff, 0xfe, b'\n']),
        None
    );
}

#[test]
fn extracts_first_line_only_for_multiline_paste() {
    // If the input box ever pastes multiple lines we still want a
    // single CommandStart for the first line.
    assert_eq!(
        PtyManager::extract_injected_command(b"first\nsecond\n"),
        Some("first".to_string())
    );
}

#[test]
fn extracts_command_terminated_by_cr() {
    // The PowerShell write path rewrites `\n` to `\r` before
    // extracting, so CR-only terminators must work too.
    assert_eq!(
        PtyManager::extract_injected_command(b"dir\r"),
        Some("dir".to_string())
    );
}

#[test]
fn lf_to_cr_translation_only_for_windows_shells() {
    assert!(PtyManager::needs_lf_to_cr_translation(
        ShellType::PowerShell
    ));
    assert!(PtyManager::needs_lf_to_cr_translation(ShellType::Cmd));
    assert!(!PtyManager::needs_lf_to_cr_translation(ShellType::Zsh));
    assert!(!PtyManager::needs_lf_to_cr_translation(ShellType::Bash));
    assert!(!PtyManager::needs_lf_to_cr_translation(ShellType::Fish));
    assert!(!PtyManager::needs_lf_to_cr_translation(ShellType::Sh));
    assert!(!PtyManager::needs_lf_to_cr_translation(ShellType::Unknown));
}

#[test]
fn translate_lf_to_cr_rewrites_bare_lf() {
    assert_eq!(
        translate_lf_to_cr_for_powershell(b"dir\n"),
        Some(b"dir\r".to_vec())
    );
}

#[test]
fn translate_lf_to_cr_preserves_crlf() {
    // `\r\n` callers (some xterm key handlers send CRLF) must reach
    // PowerShell unchanged so the CR triggers Enter and the LF is
    // ignored by PSReadLine.
    assert_eq!(translate_lf_to_cr_for_powershell(b"dir\r\n"), None);
}

#[test]
fn translate_lf_to_cr_returns_none_when_no_change_needed() {
    assert_eq!(translate_lf_to_cr_for_powershell(b"ls"), None);
    assert_eq!(translate_lf_to_cr_for_powershell(b""), None);
    assert_eq!(
        translate_lf_to_cr_for_powershell(b"already\rterminated"),
        None
    );
}

#[test]
fn translate_lf_to_cr_handles_multiple_bare_lfs() {
    // Pasting a multi-line script via the input box should turn
    // every embedded `\n` into `\r` so PSReadLine sees a sequence
    // of Enter presses rather than line-feed continuations.
    assert_eq!(
        translate_lf_to_cr_for_powershell(b"a\nb\nc\n"),
        Some(b"a\rb\rc\r".to_vec())
    );
}

#[test]
fn translate_lf_to_cr_handles_mixed_line_endings() {
    // Mixed CR / LF / CRLF input: rewrite only the bare LFs.
    assert_eq!(
        translate_lf_to_cr_for_powershell(b"a\rb\nc\r\nd\n"),
        Some(b"a\rb\rc\r\nd\r".to_vec())
    );
}
