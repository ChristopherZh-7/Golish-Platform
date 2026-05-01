/**
 * Strip ALL ANSI escape sequences (both control and color codes) from terminal output.
 * Returns plain text suitable for sending to LLMs or rendering without terminal emulation.
 */
export function stripAllAnsi(str: string): string {
  let result = str;
  // OSC sequences: ESC ] ... (BEL | ST)
  result = result.replace(/\x1b\][\s\S]*?(?:\x07|\x1b\\)/g, "");
  // CSI sequences: ESC [ ... final_byte (color codes, cursor movement, etc.)
  // biome-ignore lint/suspicious/noControlCharactersInRegex: stripping ANSI escape sequences
  result = result.replace(/\x1b\[[0-9;?]*[a-zA-Z]/g, "");
  // Bare ESC sequences (cursor save/restore, ST, etc.)
  result = result.replace(/\x1b[78\\()]/g, "");
  // Catch any remaining ESC characters
  // biome-ignore lint/suspicious/noControlCharactersInRegex: stripping leftover ESC bytes
  result = result.replace(/\x1b/g, "");
  // Remove other C0 control characters (except \n, \r, \t)
  // biome-ignore lint/suspicious/noControlCharactersInRegex: stripping control characters
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
  // When terminal outputs: content1\n\x1b[1A\rcontent2
  // It means: output line 1, newline, cursor up 1 line, carriage return, overwrite with line 2.
  // We collapse this pattern to just \r so line 1 gets properly overwritten by line 2.
  // This handles npm/pnpm/yarn multi-line progress displays.

  // Apply repeatedly to handle nested patterns (e.g., multiple consecutive overwrites)
  let prev: string;
  do {
    prev = result;
    // Pattern: newline, cursor-up (any count), optional erase sequences
    // \n\x1b[1A\r → \r (cursor up then CR to overwrite)
    // \n\x1b[1A\x1b[K → \r (cursor up then erase line)
    // \n\x1b[1A\x1b[2K\r → \r (cursor up, erase entire line, then CR)
    result = result.replace(/\n\x1b\[\d*A(?:\x1b\[\d*K)*/g, "\r");
  } while (result !== prev);

  // Strip ALL non-SGR CSI sequences while keeping \x1b[...m (colors/styles).
  // Covers cursor movement, erase, DEC private modes, bracketed paste, etc.
  // biome-ignore lint/suspicious/noControlCharactersInRegex: stripping non-SGR CSI sequences
  result = result.replace(/\x1b\[[0-9;?]*[a-ln-zA-Z]/g, "");

  // Character set selection: \x1b(B, \x1b)0, etc.
  // biome-ignore lint/suspicious/noControlCharactersInRegex: stripping character set escapes
  result = result.replace(/\x1b[()][A-Z0-9]/g, "");

  // Bare ESC sequences: cursor save/restore, ST, keypad modes, etc.
  // biome-ignore lint/suspicious/noControlCharactersInRegex: stripping bare ESC sequences
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

  return result.trim();
}
