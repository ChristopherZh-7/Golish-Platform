/**
 * When the user stops generation, the backend may still emit a few in-flight
 * `ai-event` payloads. Synchronously mark the AI session as "suppressed" so
 * the frontend ignores those events until the next `started` (new turn).
 */
const suppressedByAiSession = new Set<string>();

export function suppressGenerationForAiSession(sessionId: string): void {
  if (sessionId) suppressedByAiSession.add(sessionId);
}

export function clearGenerationSuppressForAiSession(sessionId: string): void {
  suppressedByAiSession.delete(sessionId);
}

export function isGenerationSuppressedForAiSession(sessionId: string | undefined): boolean {
  return !!sessionId && suppressedByAiSession.has(sessionId);
}
