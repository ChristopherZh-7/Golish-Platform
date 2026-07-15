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
  "SCAN_RUNNER",
  "SESSION_NOT_FOUND",
  "NOT_FOUND",
  "VALIDATION",
  "CONFIG",
  "runtime_scope_history_requires_invalidation",
  "ATTACK_CANDIDATE_PLAN_CHANGED",
  "ATTACK_REVIEW_SCOPE_MISMATCH",
  "ATTACK_APPROVAL_EXPIRED",
  "ATTACK_REVIEW_ALREADY_CLOSED",
  "ATTACK_RESUME_NOT_READY",
  "ATTACK_RECOVERY_CONFLICT",
  "STAGE_TEAM_INVALID_ID",
  "STAGE_TEAM_SCOPE_MISMATCH",
  "STAGE_TEAM_DATABASE",
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
  SCAN_RUNNER: "A scan failed to run.",
  SESSION_NOT_FOUND: "The session was not found.",
  NOT_FOUND: "The requested item was not found.",
  VALIDATION: "The input was invalid.",
  CONFIG: "There is a configuration problem.",
  runtime_scope_history_requires_invalidation:
    "This organization is retained by immutable runtime scope history. Invalidate that history before deleting it.",
  ATTACK_CANDIDATE_PLAN_CHANGED:
    "This Candidate changed after the review loaded. Refresh before deciding.",
  ATTACK_REVIEW_SCOPE_MISMATCH: "The Candidate review no longer matches this operation wave.",
  ATTACK_APPROVAL_EXPIRED: "The requested Candidate approval expiry is invalid or has passed.",
  ATTACK_REVIEW_ALREADY_CLOSED: "This Candidate review is already closed.",
  ATTACK_RESUME_NOT_READY: "The durable Candidate review is not ready to resume verification.",
  ATTACK_RECOVERY_CONFLICT:
    "The Candidate recovery state changed or the requested recovery is not valid. Refresh before deciding again.",
  STAGE_TEAM_INVALID_ID: "The Stage Team execution identity is invalid.",
  STAGE_TEAM_SCOPE_MISMATCH:
    "The Stage Team execution does not belong to this operation or operator scope.",
  STAGE_TEAM_DATABASE: "The durable Stage Team scheduler state could not be read.",
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
