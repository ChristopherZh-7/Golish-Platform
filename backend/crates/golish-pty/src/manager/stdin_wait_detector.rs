//! Heuristic detection of "the running command is blocking on stdin".
//!
//! Drives the Warp-style interactive input mode (see
//! `docs/design/2026-05-15-warp-style-interaction.md`). When the PTY has
//! been idle for at least [`STDIN_WAIT_IDLE_THRESHOLD_MS`] and the trailing
//! bytes of recent output match one of the prompt patterns implemented in
//! [`detect_stdin_wait`], the emitter thread fires a `stdin_wait` event so
//! the frontend can swap the bottom input box into "respond to this
//! command's stdin" mode.

use std::time::Duration;

/// Idle threshold before the detector even considers emitting a
/// `stdin_wait` event. Tuned to be longer than the 16 ms output coalescing
/// window so we don't false-trigger inside a coalesce batch, but short
/// enough that the UI feels responsive when a real prompt lands.
pub const STDIN_WAIT_IDLE_THRESHOLD_MS: u64 = 300;

/// Convenience [`Duration`] form of [`STDIN_WAIT_IDLE_THRESHOLD_MS`].
pub const STDIN_WAIT_IDLE_THRESHOLD: Duration =
    Duration::from_millis(STDIN_WAIT_IDLE_THRESHOLD_MS);

/// Maximum number of trailing bytes the detector inspects. Most prompt
/// markers we care about fit comfortably inside this window; longer
/// matches don't add value and would slow the regex pass.
pub const STDIN_WAIT_TAIL_BYTES: usize = 256;

/// Which heuristic fired, so the frontend (and unit tests) can tell apart
/// `[Y/n]` style prompts from `npm init` style "press Enter" prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdinWaitKind {
    /// `[Y/n]`, `[y/N]`, `[Yes] [No]`, `(yes/no)` style prompts.
    YnChoice,
    /// Password / passphrase prompt (e.g. `sudo`, `ssh`, `git push` SSH).
    Password,
    /// PowerShell `Get-Host.UI.PromptForChoice` style multi-option:
    /// `[A] Allow  [N] Never`, or `?#` shell `select` prompt.
    PowerShellChoice,
    /// "Continue?", "Are you sure?", "Press any key" style.
    Continue,
    /// Generic prompt heuristic: trailing `:` or `?` immediately preceded
    /// by non-whitespace and followed by at most one space + line end.
    /// Lower confidence than the structured patterns above.
    GenericPrompt,
}

impl StdinWaitKind {
    /// Stable string used in the `detector` field of the
    /// `stdin_wait` event payload so the frontend can map it to a
    /// human-friendly hint and tests can assert exact values.
    pub fn as_event_str(self) -> &'static str {
        match self {
            StdinWaitKind::YnChoice => "yn_choice",
            StdinWaitKind::Password => "password",
            StdinWaitKind::PowerShellChoice => "powershell_choice",
            StdinWaitKind::Continue => "continue",
            StdinWaitKind::GenericPrompt => "generic_prompt",
        }
    }
}

/// Inspect the trailing bytes of recent PTY output for prompt patterns
/// that indicate the running command is waiting on stdin.
///
/// Returns `Some(kind)` if a known prompt pattern matches the *tail* of
/// the buffer, otherwise `None`. The detector is intentionally
/// conservative — false positives turn the bottom input box into the
/// wrong mode at an inconvenient moment, which is more annoying than a
/// false negative (the user can still type into the input box normally).
///
/// All matching is case-insensitive and tolerates trailing whitespace /
/// CRLF / a single trailing cursor-positioning escape sequence
/// (e.g. `\x1b[?25h` to show the cursor) immediately after the prompt
/// text.
pub fn detect_stdin_wait(tail: &[u8]) -> Option<StdinWaitKind> {
    let text = match std::str::from_utf8(tail) {
        Ok(s) => s,
        Err(_) => {
            // Non-UTF-8 trailing bytes — bail out rather than guess.
            return None;
        }
    };

    let trimmed = strip_trailing_noise(text);
    if trimmed.is_empty() {
        return None;
    }

    // Order matters: more specific patterns first so we don't fall through
    // to the generic `:` / `?` detector when a structured match exists.
    if matches_password(trimmed) {
        return Some(StdinWaitKind::Password);
    }
    if matches_powershell_choice(trimmed) {
        return Some(StdinWaitKind::PowerShellChoice);
    }
    if matches_yn_choice(trimmed) {
        return Some(StdinWaitKind::YnChoice);
    }
    if matches_continue(trimmed) {
        return Some(StdinWaitKind::Continue);
    }
    if matches_generic_prompt(trimmed) {
        return Some(StdinWaitKind::GenericPrompt);
    }

    None
}

/// Strip trailing whitespace, line terminators, and common cursor /
/// attribute SGR sequences that real shells often emit right after a
/// prompt (e.g. show cursor `\x1b[?25h`, reset color `\x1b[0m`). Without
/// this, the regex/contains checks below would miss otherwise obvious
/// prompts.
fn strip_trailing_noise(text: &str) -> &str {
    let mut end = text.len();

    loop {
        let before = end;
        let slice = &text[..end];

        // Trim trailing ASCII whitespace (\r, \n, space, tab).
        let trimmed = slice.trim_end_matches(|c: char| c.is_ascii_whitespace());
        end = trimmed.len();

        // Drop the most common trailing ANSI sequences. We only handle a
        // handful by hand because (a) the actual set is small and (b) we
        // don't want to pull in a full ANSI parser inside this hot path.
        for suffix in &[
            "\x1b[?25h", // show cursor
            "\x1b[?25l", // hide cursor (some prompts do this)
            "\x1b[0m",   // SGR reset
            "\x1b[K",    // erase in line
            "\x1b[J",    // erase in display
        ] {
            if trimmed.ends_with(suffix) {
                end -= suffix.len();
                break;
            }
        }

        if end == before {
            break;
        }
    }

    &text[..end]
}

fn matches_yn_choice(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();

    // `[Y/n]`, `[y/N]`, `[Y/N]`, `[yes/no]`.
    if lower.contains("[y/n]") || lower.contains("[yes/no]") {
        return true;
    }

    // `(y/n)`, `(yes/no)`.
    if lower.contains("(y/n)") || lower.contains("(yes/no)") {
        return true;
    }

    // Trailing `[Y]` or `[N]` markers without enclosing brackets are
    // ambiguous (a directory listing of `[Y]es-output.log` would match);
    // require the pair to appear close together.
    let has_y = lower.contains("[y]") || lower.contains("[yes]");
    let has_n = lower.contains("[n]") || lower.contains("[no]");
    if has_y && has_n {
        return true;
    }

    false
}

fn matches_powershell_choice(text: &str) -> bool {
    // `select` builtin PS3 prompt — emitted while waiting for a
    // numeric pick. The two major shells disagree on the order:
    //
    //   bash 3.2 / 5.x  →  `#? `  (hash question-mark space)
    //   zsh  5.x       →  `?# `  (question-mark hash space)
    //
    // Both verified empirically against the per-session PTY dump
    // (see `~/.golish/backend.log` for the raw zsh capture used to
    // catch this asymmetry). An earlier revision matched only `?#`
    // (which was wrong for bash). A follow-up flipped to only `#?`
    // (which broke zsh — the user's default macOS shell). We now
    // accept both orderings; the PS3 string is short enough that the
    // false-positive surface is negligible.
    let trimmed = text.trim_end();
    if trimmed.ends_with("#?") || trimmed.ends_with("?#") {
        return true;
    }

    let lower = text.to_ascii_lowercase();

    // PowerShell `Get-Host.UI.PromptForChoice` formats the options as
    // `[A] Allow  [N] Never  [S] Suspend  [?] Help`. Detect when at
    // least three single-letter bracketed options appear near the tail.
    let bracketed_letters = count_short_bracket_options(&lower);
    if bracketed_letters >= 3 {
        return true;
    }

    // The PowerShell prompt also ends with the explicit `(default is "y")`
    // / `(default is "n")` hint just before the input cursor.
    if lower.contains("(default is \"y\")") || lower.contains("(default is \"n\")") {
        return true;
    }

    false
}

fn count_short_bracket_options(lower: &str) -> usize {
    // Count occurrences of `[a]` / `[ab]` style markers (1–3 letter
    // tokens inside square brackets). Three or more in a single tail is
    // a strong signal for PowerShell's choice prompt.
    let mut count = 0usize;
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'[' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_alphabetic() && j - i <= 4 {
                j += 1;
            }
            if j > i + 1 && j - i <= 4 && j < bytes.len() && bytes[j] == b']' {
                count += 1;
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    count
}

fn matches_password(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if (lower.ends_with(':') || lower.ends_with(": "))
        && (lower.contains("password") || lower.contains("passphrase")) {
            return true;
        }
    // SSH-style: "user@host's password:"
    if lower.contains("password:") || lower.contains("passphrase:") {
        return true;
    }
    false
}

fn matches_continue(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let needles = [
        "continue?",
        "are you sure",
        "press any key",
        "press enter to continue",
        "do you want to continue",
        "proceed with",
        "ok to proceed",
    ];
    needles.iter().any(|n| lower.contains(n))
}

fn matches_generic_prompt(text: &str) -> bool {
    let trimmed = text.trim_end();
    let last = match trimmed.chars().last() {
        Some(c) => c,
        None => return false,
    };

    if last != ':' && last != '?' && last != '>' {
        return false;
    }

    // For `>` the heuristic is intentionally looser than for `:` / `?`
    // because the leading shell continuation prompt (bash `PS2`, default
    // `> `, plus zsh `cont>` / `quote>`) sits at the start of its own
    // line and never has a preceding alphanumeric run. Recognising it
    // here is the difference between the user being able to escape a
    // half-typed compound command in the Warp input box and the input
    // box hiding behind the running-command card forever.
    //
    // For `:` / `?` we keep the stricter check so e.g. a stray `12:` in
    // a timestamp tail or a mid-sentence `?` doesn't trip the
    // interactive mode.
    let mut chars = trimmed.chars().rev();
    chars.next();
    let prev = chars.next();
    let prev_ok = match (last, prev) {
        ('>', Some(c)) => {
            c.is_alphanumeric() || c == ')' || c == ']' || c == '\n' || c == '\r'
        }
        ('>', None) => true,
        (_, Some(c)) => c.is_alphanumeric() || c == ')' || c == ']',
        (_, None) => false,
    };
    if !prev_ok {
        return false;
    }

    true
}

/// Slide the supplied tail buffer to retain only the last
/// [`STDIN_WAIT_TAIL_BYTES`] bytes plus the new chunk. Implemented as a
/// free function so the emitter-thread state struct stays tiny and so we
/// can unit-test the truncation behaviour.
pub fn append_to_tail(tail: &mut Vec<u8>, chunk: &[u8]) {
    tail.extend_from_slice(chunk);
    if tail.len() > STDIN_WAIT_TAIL_BYTES {
        let drop = tail.len() - STDIN_WAIT_TAIL_BYTES;
        tail.drain(..drop);
    }
}

#[cfg(test)]
mod tests {
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
        parser.parse_filtered(
            b"\x1b]133;C;select yn in \"Yes\" \"No\"; do echo $yn; break; done\x07",
        );
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
        assert_eq!(r.output, b"", "timeline output should stay empty in Input region");
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
}
