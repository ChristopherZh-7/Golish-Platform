/**
 * Strip ALL ANSI escape sequences (both control and color codes) from terminal output.
 * Returns plain text suitable for sending to LLMs or rendering without terminal emulation.
 */
export function stripAllAnsi(str: string): string {
  let result = str;
  // OSC sequences: ESC ] ... (BEL | ST)
  result = result.replace(/\x1b\][\s\S]*?(?:\x07|\x1b\\)/g, "");
  // CSI sequences: ESC [ ... final_byte (color codes, cursor movement, etc.)
  result = result.replace(/\x1b\[[0-9;?]*[a-zA-Z]/g, "");
  // Bare ESC sequences (cursor save/restore, ST, etc.)
  result = result.replace(/\x1b[78\\()]/g, "");
  // Catch any remaining ESC characters
  result = result.replace(/\x1b/g, "");
  // Remove other C0 control characters (except \n, \r, \t)
  result = result.replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, "");
  // Simulate carriage return (keep last overwrite segment per line)
  result = result
    .split("\n")
    .map((line) => {
      if (line.includes("\r")) {
        const segments = line.split("\r").filter((s) => s.length > 0);
        return segments.length > 0 ? segments[segments.length - 1] : "";
      }
      return line;
    })
    .join("\n");
  // Remove trailing prompt artifacts (%, $, etc.)
  result = result.replace(/\n\s*[%$>→›❯➜]\s*$/g, "");
  // Clean up blank lines at start/end
  return result.trim();
}

/**
 * Strip OSC (Operating System Command) sequences from terminal output.
 * These are control sequences like directory changes and shell integration markers,
 * not display formatting. ANSI color codes are preserved for rendering.
 */
export function stripOscSequences(str: string): string {
  // OSC sequences start with ESC ] and end with BEL (\x07) or ST (\x1b\)
  // Common OSC codes:
  // - OSC 0/1/2: Window/icon title
  // - OSC 7: Current directory
  // - OSC 133: Shell integration (prompt markers)

  let result = str;

  // Remove OSC sequences with ESC prefix: \x1b] ... (\x07 | \x1b\)
  result = result.replace(/\x1b\][\s\S]*?(?:\x07|\x1b\\)/g, "");

  // Remove OSC sequences with bare ] that might appear (defensive)
  // Match ]number; ... until we hit ESC[ (start of CSI) or end
  result = result.replace(/\](?:133|7|0|1|2|9);[^\x1b\x07]*(?:\x07|\x1b\\)?/g, "");

  // Handle cursor-up overwrite patterns BEFORE stripping CSI sequences.
  // When terminal outputs: content1\n\x1b[1A\x1b[Kcontent2
  // It means: output line 1, newline, cursor up 1 line, erase line, write line 2.
  // We collapse this pattern to just \r so line 1 gets properly overwritten by
  // line 2. This handles npm/pnpm/yarn multi-line progress displays.
  //
  // IMPORTANT: only collapse when the cursor-up is *followed by an explicit
  // erase* (\x1b[K / \x1b[2K). PowerShell's `Format-Table` and a couple of
  // PSReadLine prompt-fix paths emit a bare `\n\x1b[1A` (cursor-up with no
  // erase) to "park" the cursor without clobbering the line above; treating
  // that as an overwrite ate the entire "Mode  LastWriteTime …" header row
  // for `dir`, collapsing it into the preceding `Directory:` GroupBy line.

  // Apply repeatedly to handle nested patterns (e.g., multiple consecutive overwrites)
  let prev: string;
  do {
    prev = result;
    // Pattern: newline, cursor-up (any count), then *at least one* erase
    // sequence. We deliberately *don't* match bare `\n\x1b[Nh` cursor-up.
    // \n\x1b[1A\x1b[K  → \r (cursor up, erase line)
    // \n\x1b[1A\x1b[2K → \r (cursor up, erase entire line)
    result = result.replace(/\n\x1b\[\d*A(?:\x1b\[\d*K)+/g, "\r");
  } while (result !== prev);

  // CUP — Cursor Position. PowerShell's `Format-Table` on ConPTY uses
  // absolute cursor positioning (`\x1b[7;1H`) to lay out tables when
  // `$Host.UI.RawUI` reports a real VT terminal — it writes the GroupBy
  // header (`Directory: …`), then jumps the cursor to row 7 col 1 and
  // writes the `Mode  LastWriteTime  Length Name` row there. In a real
  // xterm the rows between are pre-existing blank screen cells; here we
  // are filling a static `<pre>` and have no grid, so the generic CSI
  // stripper below would erase `\x1b[7;1H` and weld the `Mode …` line
  // straight onto the end of `…\test`. Replacing CUP with `\n` is the
  // closest text-flow equivalent: it preserves the line break that the
  // caller intended, at the cost of one fewer blank row than a real
  // terminal would show (a totally acceptable trade-off for command
  // output in a chat-style timeline).
  //
  // CUP wire formats handled: `\x1b[H` (home), `\x1b[Nf`, `\x1b[N;MH`,
  // `\x1b[N;Mf`. The `?` on the row digits keeps `\x1b[H` matching too.
  //
  // Why `\n\n` and not `\n`: PowerShell's CUP-era Format-Table rendering
  // (first ~3 dir invocations of a fresh PSReadLine session) jumps row
  // counts in 3-line steps (`\x1b[15;1H` → `\x1b[18;1H`), assuming the
  // intervening rows are blank screen cells the user can see. Once PSReadLine
  // settles into plain-text mode (4th dir onwards), PowerShell emits a true
  // `\n\n\n` between `Directory:` and `Mode`. Replacing CUP with a single
  // `\n` makes the CUP-era output look noticeably tighter than the plain-
  // text-era output side-by-side. `\n\n` lands midway and the trailing
  // dedupe below evens both eras out to "exactly one blank line between
  // logical rows", which is what the user perceives as consistent.
  result = result.replace(/\x1b\[\d*(?:;\d*)?[Hf]/g, "\n\n");

  // Strip ALL non-SGR CSI sequences while keeping \x1b[...m (colors/styles).
  // Covers cursor movement, erase, DEC private modes, bracketed paste, etc.
  result = result.replace(/\x1b\[[0-9;?]*[a-ln-zA-Z]/g, "");

  // Character set selection: \x1b(B, \x1b)0, etc.
  result = result.replace(/\x1b[()][A-Z0-9]/g, "");

  // Bare ESC sequences: cursor save/restore, ST, keypad modes, etc.
  result = result.replace(/\x1b[78\\=>#]/g, "");

  // Simulate carriage return behavior: \r moves cursor to beginning of line,
  // so subsequent text overwrites previous content. We process line by line,
  // handling \r within each line to keep only the final visible content.
  result = result
    .split("\n")
    .map((line) => {
      // If line contains \r (not at end), split and keep only last segment
      // This simulates terminal overwrite behavior for progress bars
      if (line.includes("\r")) {
        const segments = line.split("\r");
        // Filter out empty segments and take the last non-empty one
        const nonEmpty = segments.filter((s) => s.length > 0);
        return nonEmpty.length > 0 ? nonEmpty[nonEmpty.length - 1] : "";
      }
      return line;
    })
    .join("\n");

  // Strip trailing prompt artifacts (%, $, >, etc.)
  // This handles cases where the shell prompt gets captured
  // The % is zsh's PROMPT_SP marker shown when output doesn't end with newline

  // Remove trailing prompt on its own line (with possible ANSI codes)
  result = result.replace(/\n\s*(?:\x1b\[[0-9;]*m)*[%$>→›❯➜]\s*(?:\x1b\[[0-9;]*m)*\s*$/g, "");

  // Remove standalone % at the very end (zsh PROMPT_SP)
  result = result.replace(/(?:\x1b\[[0-9;]*m)*[%]\s*(?:\x1b\[[0-9;]*m)*\s*$/g, "");

  // Clean up trailing whitespace
  result = result.replace(/\n\s*$/g, "\n");

  // Collapse 3-or-more consecutive blank lines to a single blank line.
  // PowerShell's plain-text-mode Format-Table emits `\n\n\n` between the
  // GroupBy header (`Directory: …`) and the column header (`Mode  …`),
  // while our CUP-era path produces `\n\n` (CUP → `\n\n`). Both eras
  // collapse here to "exactly one blank line", which means the user sees
  // consistent spacing whether they ran `dir` during PSReadLine warmup
  // or after it stabilised. (`\n{3,}` matches 3+ literal newlines, i.e.
  // 2+ blank lines; replacement leaves exactly 1 blank line.)
  result = result.replace(/\n{3,}/g, "\n\n");

  return result.trim();
}
