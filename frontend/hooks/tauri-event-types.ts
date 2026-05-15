// NOTE: The event payload types that were previously defined here
// (TerminalOutputEvent, CommandBlockEvent, DirectoryChangedEvent,
// VirtualEnvChangedEvent, SessionEndedEvent, AlternateScreenEvent) now
// live in `@/lib/events/payloads` as the single source of truth
// (EventPayloadMap). Consumers should use `onEvent("<channel>", ...)`
// which infers the payload type automatically — see `useTauriEvents.ts`.

export const PROCESS_DETECTION_DELAY_MS = 300;
export const SHELL_PROCESSES = new Set(["zsh", "bash", "sh", "fish"]);
export const GIT_STATUS_POLL_INTERVAL_MS = 5000;

export const FAST_COMMANDS = new Set([
  "ls",
  "pwd",
  "cd",
  "echo",
  "cat",
  "which",
  "whoami",
  "date",
  "clear",
  "exit",
  "history",
  "env",
  "printenv",
]);

/**
 * Process names that legitimately need a real terminal grid (cursor
 * positioning, alt-screen, raw mode keystroke handling). When the
 * backend reports `alternate_screen { enabled: true }` we only flip the
 * session into `fullterm` render mode if the foreground process name is
 * in this set — otherwise the alt-screen toggle is treated as noise
 * (e.g. `less | head`, a misbehaving pager, a PowerShell `Read-Host`
 * incidentally toggling cursor visibility) and we keep the Block UI so
 * the user can keep using the Warp-style interactive input box.
 *
 * Kept narrow on purpose: only programs that *cannot* function inside
 * a scrollable Block UI belong here.
 */
export const ALT_SCREEN_TUI_PROCESSES: ReadonlySet<string> = new Set([
  "vim",
  "vi",
  "nvim",
  "neovim",
  "emacs",
  "nano",
  "pico",
  "joe",
  "micro",
  "kakoune",
  "helix",
  "hx",
  "htop",
  "btop",
  "btop++",
  "atop",
  "iotop",
  "iftop",
  "top",
  "less",
  "more",
  "most",
  "man",
  "info",
  "tig",
  "lazygit",
  "lazydocker",
  "k9s",
  "ranger",
  "lf",
  "mc",
  "nnn",
  "tmux",
  "screen",
  "ssh",
  "mosh",
  "weechat",
  "irssi",
  "newsboat",
  "ncmpcpp",
  "alsamixer",
  "ncdu",
  "dialog",
  "whiptail",
]);

/**
 * @deprecated The "claude / codex / aider" command-name allowlist used
 * to auto-flip the session into fullterm mode before Warp-style
 * interactive input landed. Kept around briefly so settings consumers
 * that referenced `terminal.fullterm_commands` don't break at runtime
 * — actual fullterm activation is now driven by the alt-screen +
 * `ALT_SCREEN_TUI_PROCESSES` combination (see `useTauriEvents.ts`).
 *
 * TODO(phase-b): remove once GridTerminal replaces xterm.js and we no
 * longer need the fullterm render mode at all.
 */
export const BUILTIN_FULLTERM_COMMANDS: readonly string[] = [];

export function isFastCommand(command: string | null): boolean {
  if (!command) return true;
  const firstWord = command.trim().split(/\s+/)[0];
  return FAST_COMMANDS.has(firstWord);
}

export function shouldRefreshGitInfo(command: string | null): boolean {
  if (!command) return false;
  const trimmed = command.trim();
  if (!trimmed) return false;
  return (
    /(?:^|\s|&&|\|\||;|\()git\s+(?:checkout|switch)\b/.test(trimmed) ||
    /(?:^|\s|&&|\|\||;|\()gh\s+pr\s+checkout\b/.test(trimmed)
  );
}

/**
 * Extract the process name from a command string.
 * Returns just the base command (first word) without arguments.
 * Handles edge cases like sudo, env vars, and path prefixes.
 */
export function extractProcessName(command: string | null): string | null {
  if (!command) return null;
  const trimmed = command.trim();
  if (!trimmed) return null;
  const withoutEnv = trimmed.replace(/^[A-Z_][A-Z0-9_]*=\S+\s+/g, "");
  const withoutSudo = withoutEnv.replace(/^(sudo|doas)\s+/, "");
  const firstWord = withoutSudo.split(/\s+/)[0];
  const baseName = firstWord.split("/").pop() || firstWord;
  return baseName;
}
