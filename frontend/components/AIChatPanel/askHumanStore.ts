type PendingAskHumanLike = { requestId: string } | null | undefined;

export interface AskHumanStoreClearState {
  pendingAskHuman: Record<string, PendingAskHumanLike>;
  clearPendingAskHuman: (sessionId: string) => void;
}

export interface AskHumanIdentity {
  requestId: string;
  sessionId: string;
}

export function clearMatchingPendingAskHuman(
  state: AskHumanStoreClearState,
  request: AskHumanIdentity,
  sessionIds: Array<string | null | undefined>
) {
  const candidates = new Set<string>();
  candidates.add(request.sessionId);
  for (const sessionId of sessionIds) {
    const trimmed = sessionId?.trim();
    if (trimmed) candidates.add(trimmed);
  }

  for (const sessionId of candidates) {
    const pending = state.pendingAskHuman[sessionId];
    if (pending?.requestId === request.requestId) {
      state.clearPendingAskHuman(sessionId);
    }
  }
}
