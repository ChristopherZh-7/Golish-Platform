use super::*;

#[test]
fn detect_yn_brackets() {
    assert_eq!(
        detect_stdin_wait(b"Continue with installation [Y/n] "),
        Some(StdinWaitKind::YnChoice)
    );
    assert_eq!(
        detect_stdin_wait(b"Overwrite? [y/N]"),
        Some(StdinWaitKind::YnChoice)
    );
}

#[test]
fn detect_paren_yn() {
    assert_eq!(
        detect_stdin_wait(b"Is this OK (yes/no)? "),
        Some(StdinWaitKind::YnChoice)
    );
}

#[test]
fn detect_pair_brackets() {
    assert_eq!(
        detect_stdin_wait(b"Install module? [Yes] [No] "),
        Some(StdinWaitKind::YnChoice)
    );
}

#[test]
fn detect_powershell_choice() {
    assert_eq!(
        detect_stdin_wait(b"[A] Yes  [N] No  [S] Suspend  [?] Help\n(default is \"Y\"):"),
        Some(StdinWaitKind::PowerShellChoice)
    );
}

#[test]
fn detect_select_prompt_bash() {
    // bash `select` builtin defaults `PS3` to `#? ` (hash
    // question-mark space) — verified empirically:
    //   $ echo 'select yn in "Yes" "No"; do echo $yn; break; done' | bash
    //   1) Yes
    //   2) No
    //   #?
    assert_eq!(
        detect_stdin_wait(b"1) Yes\n2) No\n#? "),
        Some(StdinWaitKind::PowerShellChoice)
    );
}

#[test]
fn detect_select_prompt_zsh() {
    // zsh's `select` flips the order — `?# ` (question-mark hash
    // space). Captured from a real PTY dump in
    // `~/.golish/backend.log` (read_seq=39):
    //   raw_utf8=...?#
    //   raw_hex=...3f2320  (0x3f=`?`, 0x23=`#`, 0x20=space)
    // Without supporting both orderings, the headline Warp-style
    // interactive cell never engages on macOS (default shell:
    // zsh).
    assert_eq!(
        detect_stdin_wait(b"1) Yes  2) No\n?# "),
        Some(StdinWaitKind::PowerShellChoice)
    );
    // Real raw bytes include some ANSI cleanup right before the
    // prompt; the detector's `strip_trailing_noise` should still
    // peel them away.
    assert_eq!(
        detect_stdin_wait(b"1) Yes  2) No\n\x1b[0m\x1b[27m\x1b[24m\x1b[J?# "),
        Some(StdinWaitKind::PowerShellChoice)
    );
}

#[test]
fn detect_select_prompt_with_trailing_show_cursor() {
    // Some shells emit a show-cursor escape immediately after the
    // PS3 prompt (PSReadLine analogue). strip_trailing_noise needs
    // to look past it before the `#?` suffix check fires.
    assert_eq!(
        detect_stdin_wait(b"1) Yes\n2) No\n#? \x1b[?25h"),
        Some(StdinWaitKind::PowerShellChoice)
    );
}

#[test]
fn detect_bash_ps2_continuation_prompt() {
    // Bash falls back to `PS2` (default `> `) when the user enters
    // an unterminated compound command such as `select yn in "Yes"
    // "No"` (missing the `; do …; done`). The interactive shell
    // sits at this prompt waiting for more input — the very state
    // that triggers the user-reported "UI disappears" bug, since
    // the Warp input box is hidden by `isCommandRunning` and the
    // generic-prompt detector previously rejected `>` whose only
    // previous character is `\n` or start-of-tail.
    assert_eq!(
        detect_stdin_wait(b"select yn in \"Yes\" \"No\"\n> "),
        Some(StdinWaitKind::GenericPrompt)
    );
    // Tail that contains only the continuation prompt (we trimmed
    // to STDIN_WAIT_TAIL_BYTES and the earlier echo was dropped).
    assert_eq!(detect_stdin_wait(b"> "), Some(StdinWaitKind::GenericPrompt));
}

#[test]
fn detect_password() {
    assert_eq!(
        detect_stdin_wait(b"user@host's password: "),
        Some(StdinWaitKind::Password)
    );
    assert_eq!(
        detect_stdin_wait(b"Enter passphrase for key '/foo': "),
        Some(StdinWaitKind::Password)
    );
}

#[test]
fn detect_continue_pattern() {
    assert_eq!(
        detect_stdin_wait(b"Press any key to continue ..."),
        Some(StdinWaitKind::Continue)
    );
    assert_eq!(
        detect_stdin_wait(b"Are you sure you want to delete the branch?"),
        Some(StdinWaitKind::Continue)
    );
}

#[test]
fn detect_generic_prompt() {
    assert_eq!(
        detect_stdin_wait(b"Package name: "),
        Some(StdinWaitKind::GenericPrompt)
    );
    assert_eq!(
        detect_stdin_wait(b"Author: "),
        Some(StdinWaitKind::GenericPrompt)
    );
}

#[test]
fn ignore_non_prompt_output() {
    // Plain stdout from `ls`-style commands must not trigger.
    assert_eq!(detect_stdin_wait(b"file1.txt  file2.txt"), None);
    assert_eq!(detect_stdin_wait(b"Compiling foo v0.1.0"), None);
    // Trailing colon in a timestamp shouldn't trip us up.
    assert_eq!(detect_stdin_wait(b"00:12:34"), None);
    // Mid-line `?` (e.g. a question in normal prose) shouldn't fire
    // when followed by more text.
    assert_eq!(
        detect_stdin_wait(b"This question? followed by more text"),
        None
    );
    // The relaxed `>` rule must not trip on a stdout chunk where
    // `>` appears in the middle of a line (e.g. a redirect notice
    // or progress arrow followed by a space + more content).
    assert_eq!(detect_stdin_wait(b"writing > /tmp/out\nbytes: 42"), None);
    // Or where the trailing `>` is preceded by a space (e.g.
    // `running > foo` produces tail `foo` after trim — last char
    // is `o`, not `>` — but if someone emits `foo > ` we want it
    // to stay quiet because there is no prompt context.
    assert_eq!(detect_stdin_wait(b"writing arrow > "), None);
}

#[test]
fn ignore_empty_tail() {
    assert_eq!(detect_stdin_wait(b""), None);
    assert_eq!(detect_stdin_wait(b"   \n\r\n"), None);
}

#[test]
fn ignore_non_utf8_tail() {
    assert_eq!(detect_stdin_wait(&[0xff, 0xfe, 0xfd]), None);
}

#[test]
fn tolerate_show_cursor_escape() {
    // PSReadLine emits the show-cursor sequence right after a prompt.
    assert_eq!(
        detect_stdin_wait(b"Continue? [Y/n] \x1b[?25h"),
        Some(StdinWaitKind::YnChoice)
    );
}

#[test]
fn append_to_tail_truncates_oldest() {
    let mut tail = vec![b'a'; STDIN_WAIT_TAIL_BYTES];
    append_to_tail(&mut tail, b"bbb");
    assert_eq!(tail.len(), STDIN_WAIT_TAIL_BYTES);
    // Last bytes should be the most recent.
    assert_eq!(&tail[tail.len() - 3..], b"bbb");
}

#[test]
fn append_to_tail_short_history() {
    let mut tail = Vec::new();
    append_to_tail(&mut tail, b"hello");
    append_to_tail(&mut tail, b" world");
    assert_eq!(tail, b"hello world");
}

#[test]
fn end_to_end_zsh_select_lifecycle_triggers_detector() {
    // Reproduces the macOS-default-zsh capture from
    // `~/.golish/backend.log` (session 63814868, read_seq=39-40):
    //   1) Yes  2) No
    //   ?#   ← question-hash-space (NOTE: opposite order from bash)
    //   then two PromptEnd OSCs flip region back to Input
    // This was the case where the previous detector revision
    // missed the prompt entirely.
    use crate::parser::TerminalParser;

    let mut parser = TerminalParser::new();
    parser.parse_filtered(b"\x1b]133;A\x07% \x1b]133;B\x07");
    parser.parse_filtered(b"\x1b]133;C;select yn in \"Yes\" \"No\"\x07");
    let r = parser.parse_filtered(b"1) Yes  2) No\n?# ");
    // PS3 must end up in prompt_visible.
    assert!(
        r.prompt_visible.windows(3).any(|w| w == b"?# "),
        "prompt_visible should contain zsh's `?# ` PS3; got {:?}",
        String::from_utf8_lossy(&r.prompt_visible)
    );
    assert_eq!(
        detect_stdin_wait(&r.prompt_visible),
        Some(StdinWaitKind::PowerShellChoice),
        "detector should now recognise zsh's `?# ` flavour of PS3"
    );
}

#[test]
fn end_to_end_bash_select_lifecycle_triggers_detector() {
    // Drives the full pipeline (TerminalParser → prompt_visible →
    // detector) with the exact byte sequence a bash shell with
    // qbit's OSC 133 integration emits when the user submits
    //   select yn in "Yes" "No"; do echo $yn; break; done
    // Catches the regression class where the menu + PS3 land in
    // `Input` region (because zsh `zle-line-init` flipped to
    // PromptEnd) and never make it into the detector tail.
    use crate::parser::TerminalParser;

    let mut parser = TerminalParser::new();
    // Previous prompt frame: A → B (Input region).
    parser.parse_filtered(b"\x1b]133;A\x07user@host:~$ \x1b]133;B\x07");
    // preexec fires CommandStart (region flips back to Output).
    parser.parse_filtered(b"\x1b]133;C;select yn in \"Yes\" \"No\"; do echo $yn; break; done\x07");
    // bash prints the menu + PS3 prompt.
    let r1 = parser.parse_filtered(b"1) Yes\n2) No\n#? ");
    assert!(
        r1.prompt_visible.windows(3).any(|w| w == b"#? "),
        "prompt_visible from Output region should contain `#? `; got {:?}",
        String::from_utf8_lossy(&r1.prompt_visible)
    );
    assert_eq!(
        detect_stdin_wait(&r1.prompt_visible),
        Some(StdinWaitKind::PowerShellChoice),
        "detector should recognise the bash select PS3 (`#? `) prompt"
    );
}

#[test]
fn end_to_end_prompt_visible_works_even_inside_input_region() {
    // Reproduces the user-reported zsh case: an extra OSC 133;B
    // (PromptEnd) lands first (zsh `zle-line-init` quirk on every
    // PS2 readline), so the parser sits in Input region when the
    // PS3 / select menu bytes arrive. Before this refactor the
    // detector observed zero bytes; now `prompt_visible` should
    // carry the menu + `#? ` regardless of region.
    use crate::parser::TerminalParser;

    let mut parser = TerminalParser::new();
    parser.parse_filtered(b"\x1b]133;A\x07\x1b]133;B\x07");
    let r = parser.parse_filtered(b"1) Yes\n2) No\n#? ");
    assert_eq!(
        r.output, b"",
        "timeline output should stay empty in Input region"
    );
    assert!(
        r.prompt_visible.windows(3).any(|w| w == b"#? "),
        "prompt_visible should expose PS3 even from Input region; got {:?}",
        String::from_utf8_lossy(&r.prompt_visible)
    );
    assert_eq!(
        detect_stdin_wait(&r.prompt_visible),
        Some(StdinWaitKind::PowerShellChoice),
    );
}

#[test]
fn event_str_values() {
    assert_eq!(StdinWaitKind::YnChoice.as_event_str(), "yn_choice");
    assert_eq!(StdinWaitKind::Password.as_event_str(), "password");
    assert_eq!(
        StdinWaitKind::PowerShellChoice.as_event_str(),
        "powershell_choice"
    );
    assert_eq!(StdinWaitKind::Continue.as_event_str(), "continue");
    assert_eq!(
        StdinWaitKind::GenericPrompt.as_event_str(),
        "generic_prompt"
    );
}
