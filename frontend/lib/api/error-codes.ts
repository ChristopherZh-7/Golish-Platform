/**
 * Canonical Golish error codes — MIRROR of backend `GolishError::code()`
 * (backend/crates/golish/src/error.rs). Keep both in sync until P0-2's
 * ts-rs generation can derive this list automatically.
 */
export const GOLISH_ERROR_CODES = [
  "IO",
  "DATABASE",
  "JSON",
  "HTTP",
  "PTY",
  "TOOL",
  "SKILLS",
  "PENTEST",
  "VULN_INTEL",
  "PIPELINE",
  "SCAN_RUNNER",
  "SESSION_NOT_FOUND",
  "NOT_FOUND",
  "VALIDATION",
  "CONFIG",
  "INTERNAL",
] as const;

export type GolishErrorCode = (typeof GOLISH_ERROR_CODES)[number];

/** Frontend-only fallback when the backend error is not in {code,message} shape. */
export const UNKNOWN_ERROR_CODE = "UNKNOWN";

const MESSAGES: Record<GolishErrorCode | typeof UNKNOWN_ERROR_CODE, string> = {
  IO: "A file or I/O operation failed.",
  DATABASE: "A database operation failed.",
  JSON: "Failed to read or encode data.",
  HTTP: "A network request failed.",
  PTY: "A terminal session error occurred.",
  TOOL: "A tool operation failed.",
  SKILLS: "A skill operation failed.",
  PENTEST: "A pentest operation failed.",
  VULN_INTEL: "A vulnerability-intel operation failed.",
  PIPELINE: "A pipeline operation failed.",
  SCAN_RUNNER: "A scan failed to run.",
  SESSION_NOT_FOUND: "The session was not found.",
  NOT_FOUND: "The requested item was not found.",
  VALIDATION: "The input was invalid.",
  CONFIG: "There is a configuration problem.",
  INTERNAL: "An unexpected error occurred.",
  UNKNOWN: "An unexpected error occurred.",
};

/**
 * Translate a backend error `code` into a user-facing message. Falls back to
 * the raw backend `message` when the code is unknown, then to a generic line.
 */
export function translateErrorCode(code: string, fallbackMessage?: string): string {
  if (code in MESSAGES) {
    return MESSAGES[code as GolishErrorCode];
  }
  return fallbackMessage && fallbackMessage.length > 0 ? fallbackMessage : MESSAGES.UNKNOWN;
}
