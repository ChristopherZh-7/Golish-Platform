/**
 * One-shot suppression of a terminal's mount auto-focus.
 *
 * Chat-first focus: when the user opens a new tab the cursor should land in the
 * AI chat panel, not the terminal. `useCreateTerminalTab` marks the freshly
 * created session here; `Terminal`'s mount effect consumes the mark and skips
 * its single auto-focus call. Everything else (clicking the terminal, explicit
 * focus, fullterm/TUI transitions) still focuses the terminal as before.
 */

const suppressed = new Set<string>();

/** Mark a session so the terminal's next mount auto-focus is skipped once. */
export function suppressNextTerminalAutoFocus(sessionId: string): void {
  if (sessionId) suppressed.add(sessionId);
}

/**
 * Consume the suppression for a session. Returns `true` when the caller should
 * skip the mount auto-focus (and clears the mark so later focuses are normal).
 */
export function consumeTerminalAutoFocusSuppression(sessionId: string): boolean {
  return suppressed.delete(sessionId);
}
