export interface TerminalOutputEvent {
  session_id: string;
  data: string;
}

export interface CommandBlockEvent {
  session_id: string;
  command: string | null;
  exit_code: number | null;
  event_type: "prompt_start" | "prompt_end" | "command_start" | "command_end";
}

export interface DirectoryChangedEvent {
  session_id: string;
  path: string;
}

export interface VirtualEnvChangedEvent {
  session_id: string;
  name: string | null;
}

export interface SessionEndedEvent {
  sessionId: string;
}

export interface AlternateScreenEvent {
  session_id: string;
  enabled: boolean;
}

export const PROCESS_DETECTION_DELAY_MS = 300;
export const SHELL_PROCESSES = new Set(["zsh", "bash", "sh", "fish"]);
export const GIT_STATUS_POLL_INTERVAL_MS = 5000;

export const FAST_COMMANDS = new Set([
  "ls", "pwd", "cd", "echo", "cat", "which", "whoami",
  "date", "clear", "exit", "history", "env", "printenv",
]);

export const BUILTIN_FULLTERM_COMMANDS = [
  "claude", "cc", "codex", "cdx", "aider", "cursor", "gemini",
];

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
