/**
 * Time-windowed suppression of a terminal session's auto-focus.
 *
 * Chat-first focus: when the user opens a new tab the cursor should land in the
 * AI chat panel, not the terminal. A freshly created terminal grabs focus from
 * SEVERAL independent sources at DIFFERENT times during startup — the xterm
 * mount effect, and the `UnifiedInput` textarea's mount + "process finished"
 * focus effects (the prompt becomes ready a few seconds after the shell spawns).
 * A one-shot mark only stops the first of these, so focus still jumps to the
 * terminal a moment later.
 *
 * Instead we mark the session as suppressed for a startup WINDOW. Every
 * auto-focus source checks `isTerminalAutoFocusSuppressed` (non-consuming) and
 * skips while the window is open, so the cursor stays in the chat input
 * throughout startup. The focused AI chat textarea is also a hard guard: if the
 * user is actively typing in chat, terminal auto-focus must never steal focus,
 * even after a slow shell prompt becomes ready later than the startup window.
 * User-initiated focus (clicking the terminal) is never gated — it goes through
 * the DOM directly — and clears the window early via
 * `clearTerminalAutoFocusSuppression` so normal terminal focus resumes at once.
 */

const DEFAULT_WINDOW_MS = 30_000;
const AI_CHAT_INPUT_SELECTOR = "[data-ai-chat-input]";

/** Epoch-ms (Date.now) until which a session's auto-focus stays suppressed. */
const suppressedUntil = new Map<string, number>();

/**
 * Suppress a terminal session's auto-focus for `windowMs` (default ~6s) so a
 * chat-first new tab keeps the cursor in the AI chat input even as the terminal
 * and its input box each try to grab focus during shell startup.
 */
export function suppressTerminalAutoFocus(sessionId: string, windowMs = DEFAULT_WINDOW_MS): void {
  if (sessionId) suppressedUntil.set(sessionId, Date.now() + Math.max(0, windowMs));
}

function isAiChatInputFocused(): boolean {
  if (typeof document === "undefined") return false;
  const active = document.activeElement;
  return active instanceof HTMLElement && active.matches(AI_CHAT_INPUT_SELECTOR);
}

/**
 * True while the session is still inside its suppression window. Non-consuming
 * (auto-expires) so every async focus attempt during startup is covered, not
 * just the first one.
 */
export function isTerminalAutoFocusSuppressed(sessionId: string): boolean {
  if (!sessionId) return false;
  if (isAiChatInputFocused()) return true;

  const until = suppressedUntil.get(sessionId);
  if (until === undefined) return false;
  if (Date.now() >= until) {
    suppressedUntil.delete(sessionId);
    return false;
  }
  return true;
}

/** Clear suppression immediately — e.g. the user explicitly focused the terminal. */
export function clearTerminalAutoFocusSuppression(sessionId: string): void {
  suppressedUntil.delete(sessionId);
}
