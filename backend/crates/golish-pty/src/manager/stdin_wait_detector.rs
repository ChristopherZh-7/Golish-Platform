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
pub const STDIN_WAIT_IDLE_THRESHOLD: Duration = Duration::from_millis(STDIN_WAIT_IDLE_THRESHOLD_MS);

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
        && (lower.contains("password") || lower.contains("passphrase"))
    {
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
        ('>', Some(c)) => c.is_alphanumeric() || c == ')' || c == ']' || c == '\n' || c == '\r',
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
#[path = "stdin_wait_detector_tests.rs"]
mod tests;
