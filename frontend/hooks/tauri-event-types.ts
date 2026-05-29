// NOTE: The event payload types that were previously defined here
// (TerminalOutputEvent, CommandBlockEvent, DirectoryChangedEvent,
// VirtualEnvChangedEvent, SessionEndedEvent, AlternateScreenEvent) now
// live in `@/lib/events/payloads` as the single source of truth
// (EventPayloadMap). Consumers should use `onEvent("<channel>", ...)`
// which infers the payload type automatically — see
// `useTauriEvents.ts` / `services/terminal-events.ts`.

export const PROCESS_DETECTION_DELAY_MS = 300;
export const SHELL_PROCESSES = new Set(["zsh", "bash", "sh", "fish"]);

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

export const BUILTIN_FULLTERM_COMMANDS = [
  "claude",
  "cc",
  "codex",
  "cdx",
  "aider",
  "cursor",
  "gemini",
];

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
]);

export function isFastCommand(command: string | null): boolean {
  if (!command) return true;
  const firstWord = command.trim().split(/\s+/)[0];
  return FAST_COMMANDS.has(firstWord);
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
